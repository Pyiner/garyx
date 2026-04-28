use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigDiagnostics {
    #[serde(default)]
    pub errors: Vec<ConfigDiagnostic>,
    #[serde(default)]
    pub warnings: Vec<ConfigDiagnostic>,
}

impl ConfigDiagnostics {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn push_error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        path: Option<impl Into<String>>,
    ) {
        self.errors.push(ConfigDiagnostic {
            code: code.into(),
            message: message.into(),
            path: path.map(|p| p.into()),
        });
    }

    pub fn push_warning(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        path: Option<impl Into<String>>,
    ) {
        self.warnings.push(ConfigDiagnostic {
            code: code.into(),
            message: message.into(),
            path: path.map(|p| p.into()),
        });
    }
}
