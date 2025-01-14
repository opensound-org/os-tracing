use derive_more::{Display, FromStr};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU16, ops::Deref, str};

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MsgFormat {
    #[default]
    Json,
    Bincode,
    Msgpack,
}

impl MsgFormat {
    pub const fn default_rs() -> Self {
        MsgFormat::Bincode
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum QueryHistory {
    None,
    #[default]
    Full,
    #[serde(untagged)]
    Limit(NonZeroU16),
}

#[derive(Display, FromStr, Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct LinkClient(pub bool);

impl Default for LinkClient {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct ObserverOptions {
    pub query_history: QueryHistory,
    pub link_client: LinkClient,
}

pub trait QueryMap {
    fn with_token(self, token: &str) -> Self;
    fn with_msg_format(self, format: MsgFormat) -> Self;
    fn with_observer_options(self, observer_options: ObserverOptions) -> Self;

    fn get_token(&self) -> Option<String>;
    fn get_msg_format(&self) -> MsgFormat;
    fn get_observer_options(&self) -> ObserverOptions;

    fn get_or_default<'a, T: Default, E>(
        &'a self,
        key: &str,
        parse: impl FnOnce(&'a str) -> Result<T, E>,
    ) -> T;
}

impl QueryMap for IndexMap<String, String> {
    fn with_token(mut self, token: &str) -> Self {
        self.insert("token".into(), token.into());
        self
    }

    fn with_msg_format(mut self, format: MsgFormat) -> Self {
        self.insert("format".into(), ron::to_string(&format).unwrap());
        self
    }

    fn with_observer_options(mut self, observer_options: ObserverOptions) -> Self {
        self.insert(
            "history".into(),
            ron::to_string(&observer_options.query_history).unwrap(),
        );
        self.insert("link".into(), observer_options.link_client.to_string());
        self
    }

    fn get_token(&self) -> Option<String> {
        self.get("token").cloned()
    }

    fn get_msg_format(&self) -> MsgFormat {
        self.get_or_default("format", ron::from_str)
    }

    fn get_observer_options(&self) -> ObserverOptions {
        let query_history = self.get_or_default("history", ron::from_str);
        let link_client = self.get_or_default("link", str::FromStr::from_str);

        ObserverOptions {
            query_history,
            link_client,
        }
    }

    fn get_or_default<'a, T: Default, E>(
        &'a self,
        key: &str,
        parse: impl FnOnce(&'a str) -> Result<T, E>,
    ) -> T {
        self.get(key)
            .map(Deref::deref)
            .map(parse)
            .map(Result::unwrap_or_default)
            .unwrap_or_default()
    }
}
