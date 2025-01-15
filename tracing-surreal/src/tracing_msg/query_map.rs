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

pub trait QueryMap {
    fn with_token(self, token: &str) -> Self;
    fn with_msg_format(self, format: MsgFormat) -> Self;
    fn with_query_history(self, query_history: QueryHistory) -> Self;

    fn get_token(&self) -> Option<String>;
    fn get_msg_format(&self) -> MsgFormat;
    fn get_query_history(&self) -> QueryHistory;

    fn parse_or_default<'a, T>(&'a self, key: &str) -> T
    where
        T: Default + Deserialize<'a>;
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

    fn with_query_history(mut self, query_history: QueryHistory) -> Self {
        self.insert("history".into(), ron::to_string(&query_history).unwrap());
        self
    }

    fn get_token(&self) -> Option<String> {
        self.get("token").cloned()
    }

    fn get_msg_format(&self) -> MsgFormat {
        self.parse_or_default("format")
    }

    fn get_query_history(&self) -> QueryHistory {
        self.parse_or_default("history")
    }

    fn parse_or_default<'a, T>(&'a self, key: &str) -> T
    where
        T: Default + Deserialize<'a>,
    {
        self.get(key)
            .map(Deref::deref)
            .map(ron::from_str)
            .map(Result::unwrap_or_default)
            .unwrap_or_default()
    }
}
