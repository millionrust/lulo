use url::Url;

use crate::{model::Services, provider::Provider, Secret};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInError {
    InvalidTransition,
    WrongProvider,
    InvalidCallback,
    StateMismatch,
    EmptyCode,
    NoServices,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInState {
    ProviderChoice,
    PasswordEntry {
        provider: Provider,
    },
    Browser {
        provider: Provider,
        state: Secret,
    },
    Exchanging {
        provider: Provider,
    },
    CheckingPassword {
        provider: Provider,
    },
    ServiceChoice {
        provider: Provider,
        services: Services,
    },
    Complete {
        provider: Provider,
        services: Services,
    },
    Failed {
        provider: Provider,
    },
    Cancelled,
}

/// One sign-in attempt. Credentials and authorization codes are passed to the
/// adapter and never retained by this state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    state: SignInState,
}

impl Default for SignIn {
    fn default() -> Self {
        Self {
            state: SignInState::ProviderChoice,
        }
    }
}

impl SignIn {
    pub fn state(&self) -> &SignInState {
        &self.state
    }

    pub fn select_provider(&mut self, provider: Provider) -> Result<(), SignInError> {
        if !matches!(
            self.state,
            SignInState::ProviderChoice | SignInState::Failed { .. }
        ) {
            return Err(SignInError::InvalidTransition);
        }
        self.state = SignInState::PasswordEntry { provider };
        Ok(())
    }

    /// The adapter creates a cryptographically random state and PKCE verifier.
    /// It must acquire the GOA OAuth2 name before opening the browser.
    pub fn begin_oauth(&mut self, state: Secret) -> Result<(), SignInError> {
        let SignInState::PasswordEntry { provider } = self.state else {
            return Err(SignInError::InvalidTransition);
        };
        if provider.info().oauth.is_none() {
            return Err(SignInError::WrongProvider);
        }
        if state.expose().is_empty() {
            return Err(SignInError::StateMismatch);
        }
        self.state = SignInState::Browser { provider, state };
        Ok(())
    }

    /// Validate the redirect before ACC-2 exchanges its code. The caller must
    /// separately authenticate the D-Bus name owner of the callback.
    pub fn browser_return(&mut self, client_id: &str, uri: &str) -> Result<Secret, SignInError> {
        let SignInState::Browser { provider, state } = &self.state else {
            return Err(SignInError::InvalidTransition);
        };
        let oauth = provider.info().oauth.ok_or(SignInError::WrongProvider)?;
        if client_id != oauth.client_id {
            return Err(SignInError::InvalidCallback);
        }
        let expected = Url::parse(oauth.redirect_uri).map_err(|_| SignInError::InvalidCallback)?;
        let callback = Url::parse(uri).map_err(|_| SignInError::InvalidCallback)?;
        if callback.scheme() != expected.scheme()
            || callback.host_str() != expected.host_str()
            || callback.path() != expected.path()
            || callback.port() != expected.port()
            || callback.fragment().is_some()
            || callback.username() != expected.username()
            || callback.password() != expected.password()
        {
            return Err(SignInError::InvalidCallback);
        }
        let mut returned_state = None;
        let mut code = None;
        for (key, value) in callback.query_pairs() {
            match key.as_ref() {
                "state" if returned_state.is_none() => returned_state = Some(value.into_owned()),
                "code" if code.is_none() => code = Some(value.into_owned()),
                "error" => return Err(SignInError::InvalidCallback),
                "state" | "code" => return Err(SignInError::InvalidCallback),
                _ => {}
            }
        }
        if returned_state.as_deref() != Some(state.expose()) {
            return Err(SignInError::StateMismatch);
        }
        let code = code
            .filter(|value| !value.is_empty())
            .ok_or(SignInError::EmptyCode)?;
        let provider = *provider;
        self.state = SignInState::Exchanging { provider };
        Ok(Secret::new(code))
    }

    pub fn submit_password(&mut self) -> Result<(), SignInError> {
        let SignInState::PasswordEntry { provider } = self.state else {
            return Err(SignInError::InvalidTransition);
        };
        if provider.info().oauth.is_some() {
            return Err(SignInError::WrongProvider);
        }
        self.state = SignInState::CheckingPassword { provider };
        Ok(())
    }

    pub fn verified(&mut self) -> Result<(), SignInError> {
        let provider = match self.state {
            SignInState::Exchanging { provider } | SignInState::CheckingPassword { provider } => {
                provider
            }
            _ => return Err(SignInError::InvalidTransition),
        };
        self.state = SignInState::ServiceChoice {
            provider,
            services: Services::for_provider(provider),
        };
        Ok(())
    }

    pub fn fail(&mut self) -> Result<(), SignInError> {
        let provider = match self.state {
            SignInState::Browser { provider, .. }
            | SignInState::Exchanging { provider }
            | SignInState::CheckingPassword { provider } => provider,
            _ => return Err(SignInError::InvalidTransition),
        };
        self.state = SignInState::Failed { provider };
        Ok(())
    }

    pub fn set_services(&mut self, services: Services) -> Result<(), SignInError> {
        let SignInState::ServiceChoice { provider, .. } = self.state else {
            return Err(SignInError::InvalidTransition);
        };
        if provider == Provider::Other && services.contacts {
            return Err(SignInError::WrongProvider);
        }
        self.state = SignInState::ServiceChoice { provider, services };
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), SignInError> {
        let SignInState::ServiceChoice { provider, services } = self.state else {
            return Err(SignInError::InvalidTransition);
        };
        if !services.any() {
            return Err(SignInError::NoServices);
        }
        self.state = SignInState::Complete { provider, services };
        Ok(())
    }

    pub fn cancel(&mut self) {
        if !matches!(self.state, SignInState::Complete { .. }) {
            self.state = SignInState::Cancelled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_oauth_happy_path_and_redaction() {
        let mut flow = SignIn::default();
        flow.select_provider(Provider::Google).expect("choice");
        flow.begin_oauth(Secret::new("random-state".into()))
            .expect("start");
        assert!(!format!("{flow:?}").contains("random-state"));
        let oauth = Provider::Google.info().oauth.expect("OAuth provider");
        let uri = format!("{}?code=auth-code&state=random-state", oauth.redirect_uri);
        let code = flow
            .browser_return(oauth.client_id, &uri)
            .expect("callback");
        assert_eq!(code.expose(), "auth-code");
        flow.verified().expect("exchange succeeded");
        flow.finish().expect("services selected");
        assert!(matches!(
            flow.state(),
            SignInState::Complete {
                provider: Provider::Google,
                ..
            }
        ));
    }

    #[test]
    fn callback_rejects_forgery_and_duplicate_parameters() {
        let mut flow = SignIn::default();
        flow.select_provider(Provider::Microsoft).expect("choice");
        flow.begin_oauth(Secret::new("s".into())).expect("start");
        let oauth = Provider::Microsoft.info().oauth.expect("OAuth provider");
        for (client, uri, error) in [
            (
                "wrong",
                format!("{}?code=c&state=s", oauth.redirect_uri),
                SignInError::InvalidCallback,
            ),
            (
                oauth.client_id,
                "https://evil.test/?code=c&state=s".into(),
                SignInError::InvalidCallback,
            ),
            (
                oauth.client_id,
                format!("{}?code=c&state=wrong", oauth.redirect_uri),
                SignInError::StateMismatch,
            ),
            (
                oauth.client_id,
                format!("{}?code=c&code=d&state=s", oauth.redirect_uri),
                SignInError::InvalidCallback,
            ),
            (
                oauth.client_id,
                format!("{}?code=c&state=s&error=denied", oauth.redirect_uri),
                SignInError::InvalidCallback,
            ),
            (
                oauth.client_id,
                format!("{}?code=&state=s", oauth.redirect_uri),
                SignInError::EmptyCode,
            ),
        ] {
            assert_eq!(flow.browser_return(client, &uri), Err(error));
            assert!(matches!(flow.state(), SignInState::Browser { .. }));
        }
    }

    #[test]
    fn password_flow_and_service_choice() {
        let mut flow = SignIn::default();
        flow.select_provider(Provider::Other).expect("choice");
        assert_eq!(
            flow.begin_oauth(Secret::new("s".into())),
            Err(SignInError::WrongProvider)
        );
        flow.submit_password().expect("password submitted");
        flow.verified().expect("password verified");
        assert_eq!(
            flow.set_services(Services {
                mail: false,
                calendar: false,
                contacts: false
            }),
            Ok(())
        );
        assert_eq!(flow.finish(), Err(SignInError::NoServices));
        flow.set_services(Services::MAIL_ONLY).expect("mail only");
        flow.finish().expect("done");
    }

    #[test]
    fn failure_retry_and_cancel_are_terminal() {
        let mut flow = SignIn::default();
        flow.select_provider(Provider::ICloud).expect("choice");
        flow.submit_password().expect("submitted");
        flow.fail().expect("failure");
        flow.select_provider(Provider::Yahoo).expect("retry");
        flow.cancel();
        assert_eq!(flow.submit_password(), Err(SignInError::InvalidTransition));
    }
}
