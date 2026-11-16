#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ProbeFailure, StepError};
    use crate::log::RotatingLog;
    use std::collections::HashMap;

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        ClassFromName(String, String),
        Method(Handle, String, i32),
        Field(Handle, String),
        Invoke(Handle, usize, Vec<Arg>),
        ReadPtr(usize),
    }

    /// A scripted game. Direct method and field lookups never search parents, so the walker
    /// must walk the parent chain itself.
    #[derive(Default)]
    struct FakeOps {
        classes: HashMap<(String, String), Handle>,
        class_names: HashMap<Handle, String>,
        parents: HashMap<Handle, Handle>,
        methods: HashMap<(Handle, String, i32), Handle>,
        fields: HashMap<(Handle, String), Handle>,
        offsets: HashMap<Handle, usize>,
        results: HashMap<Handle, Result<usize, ()>>,
        memory: HashMap<usize, usize>,
        object_classes: HashMap<usize, Handle>,
        calls: Vec<Call>,
    }

    const CTRL: Handle = 1;
    const CONNECTION: Handle = 5;
    const CLIENT: Handle = 6;
    const SINGLETON_CLASS: Handle = 10;
    const NATIVE_PARENT: Handle = 11;
    const GET_INSTANCE: Handle = 100;
    const GET_RPC: Handle = 101;
    const GET_TRANSFER: Handle = 102;
    const GET_TRANSPORT: Handle = 103;
    const SEND: Handle = 104;
    const CONN_FIELD: Handle = 200;
    const CLIENT_FIELD: Handle = 201;
    const HANDLE_FIELD: Handle = 202;
    const SINGLETON: usize = 0x1000;
    const RPC: usize = 0x2000;
    const TRANSFER: usize = 0x3000;
    const TRANSPORT: usize = 0x4000;
    const CONN: usize = 0x5000;
    const CLIENT_OBJ: usize = 0x6000;
    const HANDLE: usize = 0xABCD;

    impl FakeOps {
        /// A complete world. `ZTransport` is in `ZCode.ZNet`, the others in `ZCode.ZRpc`;
        /// `get_Instance` is on `ZRpcCtrl`'s parent, `handle_` on `ZTcpClient`'s parent.
        fn world() -> Self {
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
}
