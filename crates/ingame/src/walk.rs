//! The walk from the game's `ZRpcCtrl` singleton to the live chat connection's native
//! handle (design doc, section 1, option A). Everything goes through [`Il2cppOps`], so the
//! steps are tested against a scripted game; `win/il2cpp.rs` is the real one.
//!
//! Classes, methods and fields are resolved once and cached (a patched game is a new
//! process); objects are never cached, each `walk` starts from the singleton.

use crate::error::{ProbeFailure, StepError};
use crate::log::LogSink;
use crate::names::*;

/// An il2cpp class, method or field pointer.
pub type Handle = usize;

/// An argument of `il2cpp_runtime_invoke`; a value type is passed by pointer to the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    U64(u64),
    /// A `ReadOnlySpan<byte>` over `len` bytes at `ptr` (the caller keeps them alive until
    /// the invoke returns).
    Span {
        ptr: usize,
        len: usize,
    },
}

/// A managed exception thrown inside an invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exception;

/// What the walk needs from the game. Lookups are direct (they may or may not search
/// parent classes: the walker does that itself).
pub trait Il2cppOps {
    /// The class in `namespace`, over every assembly of the domain.
    fn class_from_name(&mut self, namespace: &str, name: &str) -> Option<Handle>;
    fn class_parent(&mut self, class: Handle) -> Option<Handle>;
    fn class_name(&mut self, class: Handle) -> Option<String>;
    fn method(&mut self, class: Handle, name: &str, argc: i32) -> Option<Handle>;
    fn field(&mut self, class: Handle, name: &str) -> Option<Handle>;
    fn field_offset(&mut self, field: Handle) -> usize;
    fn object_class(&mut self, object: usize) -> Handle;
    /// Calls `method` on `this` (0 for a static one); the result is the returned object, 0 for null.
    fn invoke(&mut self, method: Handle, this: usize, args: &[Arg]) -> Result<usize, Exception>;
    /// The native code address of `method` (the first field of il2cpp's `MethodInfo`);
    /// 0 when it has none.
    fn method_pointer(&mut self, method: Handle) -> usize;
    /// Reads the pointer at `address`.
    fn read_ptr(&mut self, address: usize) -> usize;
}

/// A method to hook: where its code is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PumpTarget {
    /// `Class.Method`, for the log.
    pub name: String,
    pub method: Handle,
    /// The native address of the method's code.
    pub address: usize,
}

/// What a successful walk found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walked {
    /// The live `ZTcpClient` object.
    pub client: usize,
    /// Its non-zero native handle.
    pub handle: usize,
    /// `ZTcpClient.Send(1 argument)`.
    pub send_method: Handle,
}

#[derive(Clone, Copy)]
struct Resolved {
    get_instance: Handle,
    get_rpc: Handle,
    get_transfer: Handle,
    get_transport: Handle,
    send_method: Handle,
    tcp_connection: Handle,
    conn_offset: usize,
    client_offset: usize,
    handle_offset: usize,
}

pub struct Walker<O> {
    ops: O,
    resolved: Option<Result<Resolved, StepError>>,
}

/// A parent chain longer than this is a loop.
const MAX_CHAIN: usize = 16;

impl<O: Il2cppOps> Walker<O> {
    pub fn new(ops: O) -> Self {
        Self {
            ops,
            resolved: None,
        }
    }

    pub fn ops(&self) -> &O {
        &self.ops
    }

    pub fn ops_mut(&mut self) -> &mut O {
        &mut self.ops
    }

    /// Walks to the live `ZTcpClient`. Resolves (and logs) on the first call only.
    pub fn walk(&mut self, log: &mut dyn LogSink) -> Result<Walked, ProbeFailure> {
        let r = match &self.resolved {
            Some(r) => r.clone(),
            None => {
                let r = self.resolve(log);
                if let Err(e) = &r {
                    log.warn(&format!("resolution failed: {e}"));
                }
                self.resolved = Some(r.clone());
                r
            }
        };
        let r = r.map_err(ProbeFailure::Unsupported)?;

        let ops = &mut self.ops;
        let step = |ops: &mut O, method, this, args: &[Arg], name: &str| {
            ops.invoke(method, this, args)
                .map_err(|Exception| ProbeFailure::NoClient(format!("exception:{name}")))
        };
        let no_client = |detail: &str| ProbeFailure::NoClient(detail.into());

        let singleton = step(ops, r.get_instance, 0, &[], METHOD_GET_INSTANCE)?;
        if singleton == 0 {
            return Err(no_client("singleton null"));
        }
        let rpc = step(
            ops,
            r.get_rpc,
            singleton,
            &[Arg::U64(PROXY_ID_CHITCHAT)],
            METHOD_GET_RPC_BY_PROXY_ID,
        )?;
        if rpc == 0 {
            return Err(no_client("rpc null"));
        }
        let transfer = step(ops, r.get_transfer, rpc, &[], METHOD_GET_TRANSFER)?;
        if transfer == 0 {
            return Err(no_client("transfer null"));
        }
        let transport = step(ops, r.get_transport, transfer, &[], METHOD_GET_TRANSPORT)?;
        if transport == 0 {
            return Err(no_client("transport null"));
        }
        let conn = ops.read_ptr(transport + r.conn_offset);
        if conn == 0 {
            return Err(no_client("connection null"));
        }
        let class = ops.object_class(conn);
        if class != r.tcp_connection {
            return Err(match ops.class_name(class) {
                Some(name) => ProbeFailure::NoClient(format!("udp: {name}")),
                None => no_client("udp"),
            });
        }
        let client = ops.read_ptr(conn + r.client_offset);
        if client == 0 {
            return Err(no_client("client null"));
        }
        let handle = ops.read_ptr(client + r.handle_offset);
        if handle == 0 {
            return Err(no_client("handle zero"));
        }
        Ok(Walked {
            client,
            handle,
            send_method: r.send_method,
        })
    }

    /// The first of [`HOOK_CANDIDATES`] that resolves and has native code. Makes no invoke
    /// and patches nothing; resolution only.
    pub fn resolve_pump(&mut self, log: &mut dyn LogSink) -> Result<PumpTarget, StepError> {
        let mut no_code = false;
        for candidate in HOOK_CANDIDATES {
            let label = candidate.label();
            let Some(class) = candidate
                .namespaces
                .iter()
                .find_map(|ns| self.ops.class_from_name(ns, candidate.class))
            else {
                log.info(&format!("hook candidate {label}: class missing"));
                continue;
            };
            let Ok(method) = self.find_method(class, candidate.method, candidate.argc, log) else {
                log.info(&format!("hook candidate {label}: method missing"));
                continue;
            };
            let address = self.ops.method_pointer(method);
            if address == 0 {
                log.info(&format!("hook candidate {label}: no native code"));
                no_code = true;
                continue;
            }
            log.info(&format!("hook candidate {label}: code at {address:#x}"));
            return Ok(PumpTarget {
                name: label,
                method,
                address,
            });
        }
        Err(StepError::method(if no_code {
            "pump.code"
        } else {
            "pump"
        }))
    }

    fn resolve(&mut self, log: &mut dyn LogSink) -> Result<Resolved, StepError> {
        let mut classes = [0; CLASSES.len()];
        for (slot, name) in classes.iter_mut().zip(CLASSES) {
            *slot = NAMESPACES
                .iter()
                .find_map(|ns| {
                    let class = self.ops.class_from_name(ns, name)?;
                    log.info(&format!("class {ns}.{name} found"));
                    Some(class)
                })
                .ok_or_else(|| StepError::class(name))?;
        }
        let [ctrl, rpc_impl, transfer, transport, tcp_connection, tcp_client] = classes;

        let singleton = self
            .ops
            .class_parent(ctrl)
            .ok_or_else(|| StepError::class(format!("{ZRPC_CTRL}.parent")))?;
        let get_instance = self.find_method(singleton, METHOD_GET_INSTANCE, 0, log)?;
        let get_rpc = self.find_method(ctrl, METHOD_GET_RPC_BY_PROXY_ID, 1, log)?;
        let get_transfer = self.find_method(rpc_impl, METHOD_GET_TRANSFER, 0, log)?;
        let get_transport = self.find_method(transfer, METHOD_GET_TRANSPORT, 0, log)?;
        let send_method = self.find_method(tcp_client, METHOD_SEND, 1, log)?;
        let conn_offset = self.field_offset(transport, FIELD_CONN, log)?;
        let client_offset = self.field_offset(tcp_connection, FIELD_CLIENT, log)?;
        let handle_offset = self.field_offset(tcp_client, FIELD_HANDLE, log)?;
        Ok(Resolved {
            get_instance,
            get_rpc,
            get_transfer,
            get_transport,
            send_method,
            tcp_connection,
            conn_offset,
            client_offset,
            handle_offset,
        })
    }

    /// The method on `class` or the nearest parent that declares it.
    fn find_method(
        &mut self,
        class: Handle,
        name: &str,
        argc: i32,
        log: &mut dyn LogSink,
    ) -> Result<Handle, StepError> {
        let mut at = class;
        for _ in 0..MAX_CHAIN {
            if let Some(m) = self.ops.method(at, name, argc) {
                log.info(&format!("method {name}({argc}) found"));
                return Ok(m);
            }
            match self.ops.class_parent(at) {
                Some(parent) => at = parent,
                None => break,
            }
        }
        Err(StepError::method(name))
    }

    /// The offset of the field on `class` or the nearest parent that declares it.
    fn field_offset(
        &mut self,
        class: Handle,
        name: &str,
        log: &mut dyn LogSink,
    ) -> Result<usize, StepError> {
        let mut at = class;
        for _ in 0..MAX_CHAIN {
            if let Some(f) = self.ops.field(at, name) {
                let offset = self.ops.field_offset(f);
                log.info(&format!("field {name} found at offset {offset:#x}"));
                return Ok(offset);
            }
            match self.ops.class_parent(at) {
                Some(parent) => at = parent,
                None => break,
            }
        }
        Err(StepError::field(name))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::{ProbeFailure, StepError};
    use crate::log::RotatingLog;
    use std::collections::HashMap;

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) enum Call {
        ClassFromName(String, String),
        Method(Handle, String, i32),
        Field(Handle, String),
        Invoke(Handle, usize, Vec<Arg>),
        ReadPtr(usize),
    }

    /// A scripted game. Direct method and field lookups never search parents, so the walker
    /// must walk the parent chain itself.
    #[derive(Default)]
    pub(crate) struct FakeOps {
        pub(crate) classes: HashMap<(String, String), Handle>,
        pub(crate) class_names: HashMap<Handle, String>,
        pub(crate) parents: HashMap<Handle, Handle>,
        pub(crate) methods: HashMap<(Handle, String, i32), Handle>,
        pub(crate) fields: HashMap<(Handle, String), Handle>,
        pub(crate) offsets: HashMap<Handle, usize>,
        pub(crate) results: HashMap<Handle, Result<usize, ()>>,
        pub(crate) memory: HashMap<usize, usize>,
        pub(crate) object_classes: HashMap<usize, Handle>,
        pub(crate) calls: Vec<Call>,
        pub(crate) method_ptrs: HashMap<Handle, usize>,
    }

    pub(crate) const CTRL: Handle = 1;
    const CONNECTION: Handle = 5;
    pub(crate) const CLIENT: Handle = 6;
    const SINGLETON_CLASS: Handle = 10;
    const NATIVE_PARENT: Handle = 11;
    const GET_INSTANCE: Handle = 100;
    const GET_RPC: Handle = 101;
    const GET_TRANSFER: Handle = 102;
    const GET_TRANSPORT: Handle = 103;
    pub(crate) const SEND: Handle = 104;
    const CONN_FIELD: Handle = 200;
    const CLIENT_FIELD: Handle = 201;
    const HANDLE_FIELD: Handle = 202;
    pub(crate) const SINGLETON: usize = 0x1000;
    const RPC: usize = 0x2000;
    const TRANSFER: usize = 0x3000;
    const TRANSPORT: usize = 0x4000;
    const CONN: usize = 0x5000;
    pub(crate) const CLIENT_OBJ: usize = 0x6000;
    const HANDLE: usize = 0xABCD;

    impl FakeOps {
        /// A complete world. `ZTransport` is in `ZCode.ZNet`, the others in `ZCode.ZRpc`;
        /// `get_Instance` is on `ZRpcCtrl`'s parent, `handle_` on `ZTcpClient`'s parent.
        pub(crate) fn world() -> Self {
            let mut w = FakeOps::default();
            for (h, ns, name) in [
                (1, "ZCode.ZRpc", "ZRpcCtrl"),
                (2, "ZCode.ZRpc", "ZRpcImpl"),
                (3, "ZCode.ZRpc", "ZRpcTransfer"),
                (4, "ZCode.ZNet", "ZTransport"),
                (5, "ZCode.ZRpc", "ZTcpConnection"),
                (6, "ZCode.ZRpc", "ZTcpClient"),
            ] {
                w.classes.insert((ns.into(), name.into()), h);
                w.class_names.insert(h, name.into());
            }
            w.parents.insert(CTRL, SINGLETON_CLASS);
            w.parents.insert(CLIENT, NATIVE_PARENT);
            for (class, name, argc, m) in [
                (SINGLETON_CLASS, "get_Instance", 0, GET_INSTANCE),
                (CTRL, "GetRpcByProxyId", 1, GET_RPC),
                (2, "GetTransfer", 0, GET_TRANSFER),
                (3, "GetTransport", 0, GET_TRANSPORT),
                (CLIENT, "Send", 1, SEND),
            ] {
                w.methods.insert((class, name.into(), argc), m);
            }
            for (class, name, f, off) in [
                (4, "conn_", CONN_FIELD, 0x18),
                (CONNECTION, "client_", CLIENT_FIELD, 0x68),
                (NATIVE_PARENT, "handle_", HANDLE_FIELD, 0x20),
            ] {
                w.fields.insert((class, name.into()), f);
                w.offsets.insert(f, off);
            }
            for (m, obj) in [
                (GET_INSTANCE, SINGLETON),
                (GET_RPC, RPC),
                (GET_TRANSFER, TRANSFER),
                (GET_TRANSPORT, TRANSPORT),
            ] {
                w.results.insert(m, Ok(obj));
            }
            w.memory.insert(TRANSPORT + 0x18, CONN);
            w.memory.insert(CONN + 0x68, CLIENT_OBJ);
            w.memory.insert(CLIENT_OBJ + 0x20, HANDLE);
            w.object_classes.insert(CONN, CONNECTION);
            w
        }

        fn without_method(mut self, class: Handle, name: &str) -> Self {
            self.methods
                .retain(|(c, n, _), _| !(*c == class && n == name));
            self
        }
    }

    impl Il2cppOps for FakeOps {
        fn class_from_name(&mut self, namespace: &str, name: &str) -> Option<Handle> {
            self.calls
                .push(Call::ClassFromName(namespace.into(), name.into()));
            self.classes.get(&(namespace.into(), name.into())).copied()
        }
        fn class_parent(&mut self, class: Handle) -> Option<Handle> {
            self.parents.get(&class).copied()
        }
        fn class_name(&mut self, class: Handle) -> Option<String> {
            self.class_names.get(&class).cloned()
        }
        fn method(&mut self, class: Handle, name: &str, argc: i32) -> Option<Handle> {
            self.calls.push(Call::Method(class, name.into(), argc));
            self.methods.get(&(class, name.into(), argc)).copied()
        }
        fn field(&mut self, class: Handle, name: &str) -> Option<Handle> {
            self.calls.push(Call::Field(class, name.into()));
            self.fields.get(&(class, name.into())).copied()
        }
        fn field_offset(&mut self, field: Handle) -> usize {
            self.offsets[&field]
        }
        fn object_class(&mut self, object: usize) -> Handle {
            self.object_classes.get(&object).copied().unwrap_or(999)
        }
        fn invoke(
            &mut self,
            method: Handle,
            this: usize,
            args: &[Arg],
        ) -> Result<usize, Exception> {
            self.calls.push(Call::Invoke(method, this, args.to_vec()));
            self.results
                .get(&method)
                .copied()
                .unwrap_or(Ok(0))
                .map_err(|()| Exception)
        }
        fn method_pointer(&mut self, method: Handle) -> usize {
            self.method_ptrs.get(&method).copied().unwrap_or(0)
        }
        fn read_ptr(&mut self, address: usize) -> usize {
            self.calls.push(Call::ReadPtr(address));
            self.memory.get(&address).copied().unwrap_or(0)
        }
    }

    fn walk(w: &mut Walker<FakeOps>) -> Result<Walked, ProbeFailure> {
        w.walk(&mut RotatingLog::inert())
    }

    fn walker(ops: FakeOps) -> Walker<FakeOps> {
        Walker::new(ops)
    }

    fn invokes(w: &Walker<FakeOps>) -> usize {
        w.ops()
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Invoke(..)))
            .count()
    }

    #[test]
    fn a_complete_world_walks_to_the_handle() {
        let mut w = walker(FakeOps::world());
        let walked = walk(&mut w).unwrap();
        assert_eq!(walked.handle, HANDLE);
        assert_eq!(walked.client, CLIENT_OBJ);
        assert_eq!(walked.send_method, SEND);
        let steps: Vec<Call> = w
            .ops()
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Invoke(..) | Call::ReadPtr(_)))
            .cloned()
            .collect();
        assert_eq!(
            steps,
            vec![
                Call::Invoke(GET_INSTANCE, 0, vec![]),
                Call::Invoke(GET_RPC, SINGLETON, vec![Arg::U64(1321197368)]),
                Call::Invoke(GET_TRANSFER, RPC, vec![]),
                Call::Invoke(GET_TRANSPORT, TRANSFER, vec![]),
                Call::ReadPtr(TRANSPORT + 0x18),
                Call::ReadPtr(CONN + 0x68),
                Call::ReadPtr(CLIENT_OBJ + 0x20),
            ]
        );
    }

    #[test]
    fn resolution_failures_come_before_any_invoke() {
        let cases: Vec<(FakeOps, StepError)> = vec![
            (
                {
                    let mut w = FakeOps::world();
                    w.classes.retain(|(_, n), _| n != "ZTcpClient");
                    w
                },
                StepError::class("ZTcpClient"),
            ),
            (
                FakeOps::world().without_method(3, "GetTransport"),
                StepError::method("GetTransport"),
            ),
            (
                {
                    let mut w = FakeOps::world();
                    w.fields.remove(&(CONNECTION, "client_".to_string()));
                    w
                },
                StepError::field("client_"),
            ),
            (
                FakeOps::world().without_method(CLIENT, "Send"),
                StepError::method("Send"),
            ),
            (
                {
                    let mut w = FakeOps::world();
                    w.parents.remove(&CTRL);
                    w
                },
                StepError::class("ZRpcCtrl.parent"),
            ),
            (
                FakeOps::world().without_method(SINGLETON_CLASS, "get_Instance"),
                StepError::method("get_Instance"),
            ),
        ];
        for (ops, expected) in cases {
            let mut w = walker(ops);
            assert_eq!(
                walk(&mut w),
                Err(ProbeFailure::Unsupported(expected.clone())),
                "{expected}"
            );
            assert_eq!(invokes(&w), 0, "{expected}");
        }
    }

    #[test]
    fn a_class_is_found_in_either_namespace_or_is_missing() {
        // ZTransport is in ZCode.ZNet only in the world: the walk resolves it.
        assert!(walk(&mut walker(FakeOps::world())).is_ok());
        let mut w = FakeOps::world();
        w.classes.retain(|(_, n), _| n != "ZTransport");
        assert_eq!(
            walk(&mut walker(w)),
            Err(ProbeFailure::Unsupported(StepError::class("ZTransport")))
        );
    }

    #[test]
    fn methods_and_fields_on_a_parent_are_found() {
        // In the world get_Instance and handle_ are declared on parents and the fake does
        // not search them: the walk must.
        let mut w = walker(FakeOps::world());
        assert!(walk(&mut w).is_ok());
        assert!(w
            .ops()
            .calls
            .contains(&Call::Method(SINGLETON_CLASS, "get_Instance".into(), 0)));
        assert!(w
            .ops()
            .calls
            .contains(&Call::Field(NATIVE_PARENT, "handle_".into())));
    }

    fn no_client(w: &mut Walker<FakeOps>, detail: &str) {
        match walk(w) {
            Err(ProbeFailure::NoClient(d)) => assert!(d.contains(detail), "{d} lacks {detail}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn no_client_outcomes_stop_at_their_step() {
        let mut ops = FakeOps::world();
        ops.results.insert(GET_INSTANCE, Ok(0));
        let mut w = walker(ops);
        no_client(&mut w, "singleton null");
        assert_eq!(invokes(&w), 1);

        let mut ops = FakeOps::world();
        ops.results.insert(GET_RPC, Ok(0));
        let mut w = walker(ops);
        no_client(&mut w, "rpc null");
        assert_eq!(invokes(&w), 2);

        let mut ops = FakeOps::world();
        ops.object_classes.insert(CONN, 77);
        ops.class_names.insert(77, "ZUdpConnection".into());
        no_client(&mut walker(ops), "udp");
        let mut ops = FakeOps::world();
        ops.object_classes.insert(CONN, 77);
        ops.class_names.insert(77, "ZUdpConnection".into());
        no_client(&mut walker(ops), "ZUdpConnection");

        let mut ops = FakeOps::world();
        ops.memory.insert(CONN + 0x68, 0);
        no_client(&mut walker(ops), "client null");

        let mut ops = FakeOps::world();
        ops.memory.insert(CLIENT_OBJ + 0x20, 0);
        no_client(&mut walker(ops), "handle zero");

        let mut ops = FakeOps::world();
        ops.results.insert(GET_TRANSFER, Err(()));
        let mut w = walker(ops);
        no_client(&mut w, "exception:GetTransfer");
        assert_eq!(invokes(&w), 3);
    }

    #[test]
    fn resolution_is_cached_and_objects_are_not() {
        let mut w = walker(FakeOps::world());
        walk(&mut w).unwrap();
        w.ops_mut().calls.clear();
        walk(&mut w).unwrap();
        let calls = &w.ops().calls;
        assert!(!calls.iter().any(|c| matches!(
            c,
            Call::ClassFromName(..) | Call::Method(..) | Call::Field(..)
        )));
        assert!(calls.contains(&Call::Invoke(GET_INSTANCE, 0, vec![])));
    }

    #[test]
    fn an_unsupported_game_is_not_resolved_again() {
        let mut w = walker(FakeOps::world().without_method(3, "GetTransport"));
        let first = walk(&mut w);
        w.ops_mut().calls.clear();
        assert_eq!(walk(&mut w), first);
        assert!(w.ops().calls.is_empty());
    }

    const PANDA_GAME: Handle = 20;
    const CTRL_UPDATE: Handle = 300;
    const GAME_UPDATE: Handle = 301;

    /// `world()` plus `Panda.Core.Game` and an `Update` on it and on `ZRpcCtrl`'s parent.
    fn with_pump_candidates() -> FakeOps {
        let mut w = FakeOps::world();
        w.classes
            .insert(("Panda.Core".into(), "Game".into()), PANDA_GAME);
        w.methods
            .insert((SINGLETON_CLASS, "Update".into(), 0), CTRL_UPDATE);
        w.methods
            .insert((PANDA_GAME, "Update".into(), 0), GAME_UPDATE);
        w.method_ptrs.insert(CTRL_UPDATE, 0x7000);
        w.method_ptrs.insert(GAME_UPDATE, 0x8000);
        w
    }

    fn pump(w: &mut Walker<FakeOps>) -> Result<PumpTarget, StepError> {
        w.resolve_pump(&mut RotatingLog::inert())
    }

    #[test]
    fn the_primary_pump_candidate_wins() {
        let mut w = walker(with_pump_candidates());
        let target = pump(&mut w).unwrap();
        assert_eq!(
            target,
            PumpTarget {
                name: "ZRpcCtrl.Update".into(),
                method: CTRL_UPDATE,
                address: 0x7000
            }
        );
        assert_eq!(invokes(&w), 0);
    }

    #[test]
    fn the_fallback_is_used_when_only_it_resolves() {
        let mut ops = with_pump_candidates();
        ops.methods.remove(&(SINGLETON_CLASS, "Update".into(), 0));
        let mut w = walker(ops);
        let target = pump(&mut w).unwrap();
        assert_eq!(
            (target.name.as_str(), target.method, target.address),
            ("Game.Update", GAME_UPDATE, 0x8000)
        );
        assert_eq!(invokes(&w), 0);
    }

    #[test]
    fn no_candidate_is_method_missing_pump() {
        let mut w = walker(FakeOps::world());
        assert_eq!(pump(&mut w), Err(StepError::method("pump")));
    }

    #[test]
    fn a_zero_native_pointer_is_method_missing_pump_code() {
        let mut ops = with_pump_candidates();
        ops.method_ptrs.clear();
        let mut w = walker(ops);
        assert_eq!(pump(&mut w), Err(StepError::method("pump.code")));
        assert_eq!(invokes(&w), 0);
    }

    #[test]
    fn a_zero_pointer_on_the_primary_falls_to_the_next() {
        let mut ops = with_pump_candidates();
        ops.method_ptrs.remove(&CTRL_UPDATE);
        let mut w = walker(ops);
        assert_eq!(pump(&mut w).unwrap().name, "Game.Update");
    }
}
