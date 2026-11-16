//! The names the DLL looks up in the game, from Stellar's release_3.7 dump (facts only).
//! The game's IL2CPP API is found by `GetProcAddress` on `GameAssembly.dll`, never by
//! signature scan.

/// Exports the DLL cannot work without; a missing one is `export_missing:<name>`.
pub const REQUIRED_EXPORTS: [&str; 12] = [
    "il2cpp_domain_get",
    "il2cpp_domain_get_assemblies",
    "il2cpp_assembly_get_image",
    "il2cpp_class_from_name",
    "il2cpp_class_get_parent",
    "il2cpp_class_get_method_from_name",
    "il2cpp_class_get_field_from_name",
    "il2cpp_field_get_offset",
    "il2cpp_field_static_get_value",
    "il2cpp_runtime_invoke",
    "il2cpp_object_get_class",
    "il2cpp_thread_attach",
];

/// Exports that only name a class in a log line or detach the thread; a missing one is
/// logged, never fatal.
pub const OPTIONAL_EXPORTS: [&str; 3] = [
    "il2cpp_class_get_namespace",
    "il2cpp_class_get_name",
    "il2cpp_thread_detach",
];

/// The proxy id `GetRpcByProxyId` takes for the chat rpc.
pub const PROXY_ID_CHITCHAT: u64 = 1321197368;

/// Namespaces each class is tried in, in this order.
pub const NAMESPACES: [&str; 2] = ["ZCode.ZRpc", "ZCode.ZNet"];

pub const ZRPC_CTRL: &str = "ZRpcCtrl";
pub const ZRPC_IMPL: &str = "ZRpcImpl";
pub const ZRPC_TRANSFER: &str = "ZRpcTransfer";
pub const ZTRANSPORT: &str = "ZTransport";
pub const ZTCP_CONNECTION: &str = "ZTcpConnection";
pub const ZTCP_CLIENT: &str = "ZTcpClient";

/// The six classes of the walk.
pub const CLASSES: [&str; 6] = [
    ZRPC_CTRL,
    ZRPC_IMPL,
    ZRPC_TRANSFER,
    ZTRANSPORT,
    ZTCP_CONNECTION,
    ZTCP_CLIENT,
];

pub const METHOD_GET_INSTANCE: &str = "get_Instance";
pub const METHOD_GET_RPC_BY_PROXY_ID: &str = "GetRpcByProxyId";
pub const METHOD_GET_TRANSFER: &str = "GetTransfer";
pub const METHOD_GET_TRANSPORT: &str = "GetTransport";
pub const METHOD_SEND: &str = "Send";

pub const FIELD_CONN: &str = "conn_";
pub const FIELD_CLIENT: &str = "client_";
pub const FIELD_HANDLE: &str = "handle_";

/// A per-frame method of the game's main thread the pump can hook (investigation a of the
/// plan: the names are inferred from Stellar's notes, NOT VERIFIED until Kade's run). The
/// first that resolves, has a native pointer and can be patched is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookCandidate {
    pub class: &'static str,
    pub namespaces: &'static [&'static str],
    pub method: &'static str,
    pub argc: i32,
}

impl HookCandidate {
    /// `Class.Method`, for the log and the install detail.
    pub fn label(&self) -> String {
        format!("{}.{}", self.class, self.method)
    }
}

/// The game's `Update` takes the frame time (`Update(float)`), so argc is 1; the hook never reads it.
pub const HOOK_CANDIDATES: [HookCandidate; 2] = [
    HookCandidate {
        class: ZRPC_CTRL,
        namespaces: &NAMESPACES,
        method: "Update",
        argc: 1,
    },
    HookCandidate {
        class: "Game",
        namespaces: &["Panda.Core"],
        method: "Update",
        argc: 1,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports() {
        assert_eq!(
            REQUIRED_EXPORTS,
            [
                "il2cpp_domain_get",
                "il2cpp_domain_get_assemblies",
                "il2cpp_assembly_get_image",
                "il2cpp_class_from_name",
                "il2cpp_class_get_parent",
                "il2cpp_class_get_method_from_name",
                "il2cpp_class_get_field_from_name",
                "il2cpp_field_get_offset",
                "il2cpp_field_static_get_value",
                "il2cpp_runtime_invoke",
                "il2cpp_object_get_class",
                "il2cpp_thread_attach",
            ]
        );
        assert_eq!(
            OPTIONAL_EXPORTS,
            [
                "il2cpp_class_get_namespace",
                "il2cpp_class_get_name",
                "il2cpp_thread_detach"
            ]
        );
        let mut all: Vec<_> = REQUIRED_EXPORTS.iter().chain(&OPTIONAL_EXPORTS).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 15);
    }

    #[test]
    fn walk_names() {
        assert_eq!(PROXY_ID_CHITCHAT, 1321197368);
        assert_eq!(
            CLASSES,
            [
                "ZRpcCtrl",
                "ZRpcImpl",
                "ZRpcTransfer",
                "ZTransport",
                "ZTcpConnection",
                "ZTcpClient"
            ]
        );
        assert_eq!(NAMESPACES, ["ZCode.ZRpc", "ZCode.ZNet"]);
        assert_eq!(METHOD_GET_INSTANCE, "get_Instance");
        assert_eq!(METHOD_GET_RPC_BY_PROXY_ID, "GetRpcByProxyId");
        assert_eq!(METHOD_GET_TRANSFER, "GetTransfer");
        assert_eq!(METHOD_GET_TRANSPORT, "GetTransport");
        assert_eq!(METHOD_SEND, "Send");
        assert_eq!(FIELD_CONN, "conn_");
        assert_eq!(FIELD_CLIENT, "client_");
        assert_eq!(FIELD_HANDLE, "handle_");
    }

    #[test]
    fn hook_candidates() {
        assert_eq!(HOOK_CANDIDATES.len(), 2);
        let [first, second] = HOOK_CANDIDATES;
        assert_eq!(
            (first.class, first.namespaces, first.method, first.argc),
            ("ZRpcCtrl", &["ZCode.ZRpc", "ZCode.ZNet"][..], "Update", 1)
        );
        assert_eq!(
            (second.class, second.namespaces, second.method, second.argc),
            ("Game", &["Panda.Core"][..], "Update", 1)
        );
        assert_eq!(first.label(), "ZRpcCtrl.Update");
        assert_eq!(second.label(), "Game.Update");
    }
}
