use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

string_id!(ProjectId);
string_id!(NodeId);
string_id!(PortId);
string_id!(ResourceId);
string_id!(SubscriptionId);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortKey {
    node_id: NodeId,
    port_id: PortId,
}

impl PortKey {
    pub fn new(node_id: NodeId, port_id: PortId) -> Self {
        Self { node_id, port_id }
    }

    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    pub fn port_id(&self) -> &PortId {
        &self.port_id
    }
}
