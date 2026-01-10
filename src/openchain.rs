use std::{collections::HashMap, future::Future};

use alloy::{
    json_abi::{Event, Function},
    primitives::Selector,
};
use reqwest::Client;
use revm::primitives::B256;
use serde::{Deserialize, Serialize};

use crate::fourbyte::FourBytesProvider;

const OPENCHAINS_LOOKUP_URL: &str = "https://api.openchain.xyz/signature-database/v1/lookup";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenChainItem {
    pub name: String,
    pub filtered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenChainResult {
    pub event: HashMap<B256, Option<Vec<OpenChainItem>>>,
    pub function: HashMap<Selector, Option<Vec<OpenChainItem>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenChainClientResponse {
    pub ok: bool,
    pub result: OpenChainResult,
}

#[derive(Debug, Clone)]
pub struct OpenChainClient {
    client: Client,
}

impl Default for OpenChainClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenChainClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn fetch_function_signatures(
        &self,
        sig: Selector,
    ) -> Result<Vec<Function>, color_eyre::Report> {
        let url = format!("{}?function={}&filter=true", OPENCHAINS_LOOKUP_URL, &sig);
        tracing::debug!("Requesting {} for functions signatures", &url);
        let text = self.client.get(url).send().await?;

        let resp: OpenChainClientResponse = text.json().await?;

        Ok(resp
            .result
            .function
            .get(&sig)
            .cloned()
            .flatten()
            .unwrap_or_default()
            .into_iter()
            .map(|it| Function::parse(&it.name))
            .collect::<Result<Vec<Function>, _>>()?)
    }

    pub async fn fetch_event_signatures(
        &self,
        sig: B256,
    ) -> Result<Vec<Event>, color_eyre::Report> {
        let url = format!("{}?event={}&filtered=true", OPENCHAINS_LOOKUP_URL, sig);
        tracing::debug!("Requesting {} for event signatures", &url);
        let text = self.client.get(url).send().await?;

        let resp: OpenChainClientResponse = text.json().await?;

        Ok(resp
            .result
            .event
            .get(&sig)
            .cloned()
            .flatten()
            .unwrap_or_default()
            .into_iter()
            .map(|it| Event::parse(&it.name))
            .collect::<Result<Vec<Event>, _>>()?)
    }
}

impl FourBytesProvider for OpenChainClient {
    fn event_signatures(
        &self,
        topic: B256,
    ) -> impl Future<Output = Result<Vec<Event>, color_eyre::Report>> {
        async move { self.fetch_event_signatures(topic).await }
    }

    fn function_signatures(
        &self,
        selector: Selector,
    ) -> impl Future<Output = Result<Vec<Function>, color_eyre::Report>> {
        async move { self.fetch_function_signatures(selector).await }
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use alloy::primitives::Selector;
    use revm::primitives::b256;

    use crate::openchain::OpenChainClient;

    #[tokio::test]
    async fn test_func() {
        let client = OpenChainClient::new();

        let func = client
            .fetch_function_signatures(Selector::from_str("0x722713f7").unwrap())
            .await
            .unwrap();

        assert!(func.len() > 0);
    }

    #[tokio::test]
    async fn test_event() {
        let client = OpenChainClient::new();

        let event = client
            .fetch_event_signatures(b256!(
                "a8b21f72cb83bce808df32dc2330217d744a1c22f3e9e44e4b11bbf049d37d9d"
            ))
            .await
            .unwrap();

        assert!(event.len() > 0);
    }
}
