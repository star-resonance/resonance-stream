//! Why a lookup or the walk failed, and how each failure reaches the pipe.

use resonance_types::ingame_pipe::ErrorCode;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Export,
    Class,
    Method,
    Field,
}

/// Something the DLL could not resolve in the game: the game build differs from what the
/// DLL knows. Always answered `unsupported`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepError {
    kind: Kind,
    name: String,
}

impl StepError {
    pub fn export(name: impl Into<String>) -> Self {
        Self::new(Kind::Export, name)
    }
    pub fn class(name: impl Into<String>) -> Self {
        Self::new(Kind::Class, name)
    }
    pub fn method(name: impl Into<String>) -> Self {
        Self::new(Kind::Method, name)
    }
    pub fn field(name: impl Into<String>) -> Self {
        Self::new(Kind::Field, name)
    }

    fn new(kind: Kind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self.kind {
            Kind::Export => "export_missing",
            Kind::Class => "class_missing",
            Kind::Method => "method_missing",
            Kind::Field => "field_missing",
        }
    }

    /// `<code>:<name>`, the reply's `detail`.
    pub fn detail(&self) -> String {
        format!("{}:{}", self.code(), self.name)
    }

    pub fn error_code(&self) -> ErrorCode {
        ErrorCode::Unsupported
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail())
    }
}

/// Why the probe (the walk to the live `ZTcpClient`) did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeFailure {
    /// The game is up but has no usable chat connection yet (the detail says where the
    /// walk stopped).
    NoClient(String),
    /// The game's build lacks something the DLL needs.
    Unsupported(StepError),
}

impl ProbeFailure {
    pub fn error_code(&self) -> ErrorCode {
        match self {
            ProbeFailure::NoClient(_) => ErrorCode::NoClient,
            ProbeFailure::Unsupported(e) => e.error_code(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            ProbeFailure::NoClient(d) => d.clone(),
            ProbeFailure::Unsupported(e) => e.detail(),
        }
    }
}

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
