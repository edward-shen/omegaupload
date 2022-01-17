use omegaupload_common::secrecy::{ExposeSecret, SecretString};

pub struct Builder {
    decryption_key: SecretString,
    needs_password: bool,
    file_name: Option<String>,
    language: Option<String>,
}

impl Builder {
    pub fn new(decryption_key: SecretString) -> Self {
        Self {
            decryption_key,
            needs_password: false,
            file_name: None,
            language: None,
        }
    }

    pub const fn needs_password(mut self) -> Self {
        self.needs_password = true;
        self
    }

    // False positive
    #[allow(clippy::missing_const_for_fn)]
    pub fn file_name(mut self, name: String) -> Self {
        self.file_name = Some(name);
        self
    }

    // False positive
    #[allow(clippy::missing_const_for_fn)]
    pub fn language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    pub fn build(self) -> SecretString {
        if !self.needs_password && self.file_name.is_none() && self.language.is_none() {
            return self.decryption_key;
        }
        let mut args = String::new();
        if self.needs_password {
            args.push_str("!pw");
        }
        if let Some(file_name) = self.file_name {
            args.push_str("!name:");
            args.push_str(&file_name);
        }
        if let Some(language) = self.language {
            args.push_str("!lang:");
            args.push_str(&language);
        }
        SecretString::new(format!(
            "key:{}{}",
            self.decryption_key.expose_secret(),
            args
        ))
    }
}
