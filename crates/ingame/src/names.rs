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
}
