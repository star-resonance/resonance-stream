#[cfg(test)]
mod tests {
    use super::*;
    use resonance_types::ingame_pipe::ErrorCode;

    #[test]
    fn step_errors() {
        let cases = [
            (
                StepError::export("il2cpp_domain_get"),
                "export_missing",
                "export_missing:il2cpp_domain_get",
            ),
            (
                StepError::class("ZTcpClient"),
                "class_missing",
                "class_missing:ZTcpClient",
            ),
            (
                StepError::method("Send"),
                "method_missing",
                "method_missing:Send",
            ),
            (
                StepError::field("client_"),
                "field_missing",
                "field_missing:client_",
            ),
        ];
        for (e, code, detail) in cases {
            assert_eq!(e.code(), code);
            assert_eq!(e.detail(), detail);
            assert_eq!(e.error_code(), ErrorCode::Unsupported);
        }
    }

    #[test]
    fn probe_failures_map_to_codes() {
        let f = ProbeFailure::NoClient("udp".into());
        assert_eq!(
            (f.error_code(), f.detail()),
            (ErrorCode::NoClient, "udp".to_string())
        );
        let f = ProbeFailure::Unsupported(StepError::method("GetTransport"));
        assert_eq!(
            (f.error_code(), f.detail()),
            (
                ErrorCode::Unsupported,
                "method_missing:GetTransport".to_string()
            )
        );
    }
}
