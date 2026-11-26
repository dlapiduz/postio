//! What the digest rules list asks of the host, and its answer.

use postio_client::Client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Waiting(Vec<String>),
    DeleteRule(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Waiting {
        names: Vec<String>,
        holds: Result<Vec<u32>, String>,
    },
    RuleDeleted {
        name: String,
        released: Result<u32, String>,
    },
}

impl Ask {
    pub async fn perform(self, client: &Client) -> Answer {
        let said = |error: postio_model::listing::StoreError| error.message().to_owned();
        match self {
            Ask::Waiting(names) => Answer::Waiting {
                holds: client.digest_waiting(names.clone()).await.map_err(said),
                names,
            },
            Ask::DeleteRule(name) => Answer::RuleDeleted {
                released: client.delete_digest_rule(name.clone()).await.map_err(said),
                name,
            },
        }
    }
}
