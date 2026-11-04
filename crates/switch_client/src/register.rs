//! POST /agents with an always_on IntegrationProfile.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Error, decode, http_client};

/// What registration returns. The API key is shown once; store it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub agent_id: String,
    pub api_key: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("agent_id", &self.agent_id)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct Registration {
    /// Matches `^[a-z0-9][a-z0-9._-]*$`.
    pub name: String,
    pub description: String,
    pub display_name: Option<String>,
    /// Free-form label shown in agent details.
    pub connector_type: String,
    pub overwrite: bool,
}

#[derive(Deserialize)]
struct Registered {
    id: String,
    api_key: String,
}

/// Authenticates with the registration token (an API key will not register).
pub async fn register(
    base_url: &str,
    registration_token: &str,
    reg: &Registration,
) -> Result<Credentials, Error> {
    let body = json!({
        "name": reg.name,
        "description": reg.description,
        "display_name": reg.display_name,
        "connector_type": reg.connector_type,
        "integration_profile": {
            "connection_model": "always_on",
            "message_exchange": true,
            "pre_invocation_mediation": [],
            "post_invocation_mediation": [],
            "event_reporting": [],
        },
        "overwrite": reg.overwrite,
    });
    let url = format!("{}/agents", base_url.trim_end_matches('/'));
    let resp = http_client()?
        .post(url)
        .bearer_auth(registration_token)
        .json(&body)
        .send()
        .await?;
    let r: Registered = decode(resp).await?;
    Ok(Credentials {
        agent_id: r.id,
        api_key: r.api_key,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn debug_redacts_the_key() {
        let c = super::Credentials {
            agent_id: "a".into(),
            api_key: "secret".into(),
        };
        assert!(!format!("{c:?}").contains("secret"));
    }
}
