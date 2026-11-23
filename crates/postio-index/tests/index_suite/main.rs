//! One binary for `postio-index`'s integration tests: each file in `tests/` links the whole
//! dependency stack, so cases share one link (#1128).
//! A case that needs its own process (sets a process-global) does not belong here.

mod account_scope;
mod attachment_text;
mod backfill;
mod body_index;
mod completions;
mod contact_aware_search;
mod content_membership;
mod conversations;
mod digest_matcher;
mod driven_join_plan;
mod exact_copies;
mod executor;
mod facets_one_pass;
mod group_filter;
mod header_index;
mod header_index_size;
mod label_and_action;
mod promoted_operators;
mod ranking_weights;
mod relaxations;
mod schema_upgrade;
mod search_statement_budget;
mod startup_index_budget;
mod suggestion;
mod total_hits_cap;
mod value_sets;
mod with_filter;
