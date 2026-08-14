//! Docker-style random pane names: `{adjective}_{noun}`.
//!
//! Theme: nerd + mildly cursed developer humor.

use std::collections::HashSet;

const ADJECTIVES: &[&str] = &[
    "async",
    "atomic",
    "buffered",
    "cached",
    "caffeinated",
    "compiled",
    "cursed",
    "detached",
    "fuzzy",
    "heisen",
    "idempotent",
    "leaky",
    "mutable",
    "nested",
    "orthogonal",
    "overcaffeinated",
    "parallel",
    "quantum",
    "recursive",
    "segfaulted",
    "sideways",
    "sticky",
    "undead",
    "untyped",
    "verbose",
    "zombie",
    "bitwise",
    "branched",
    "crashed",
    "debug",
    "epochal",
    "forked",
    "ghosted",
    "hashed",
    "inlined",
    "janky",
    "kernel",
    "linted",
    "memoized",
    "nonblocking",
    "offbyone",
    "pipelined",
    "quux",
    "rebased",
    "sandboxed",
    "thunky",
    "unmerged",
    "virtual",
    "wired",
    "xenial",
    "yielding",
];

const NOUNS: &[&str] = &[
    "turing",
    "knuth",
    "hopper",
    "lovelace",
    "torvalds",
    "ritchie",
    "thompson",
    "lamport",
    "mccarthy",
    "kay",
    "rubberduck",
    "yak",
    "heisenbug",
    "nullptr",
    "stacktrace",
    "bikeshed",
    "rebase",
    "monad",
    "functor",
    "pipeline",
    "daemon",
    "orphan",
    "inode",
    "pixel",
    "sprite",
    "segfault",
    "coredump",
    "pager",
    "tty",
    "shell",
    "prompt",
    "cursor",
    "buffer",
    "socket",
    "mutex",
    "latch",
    "quark",
    "bitflip",
    "nanite",
    "gremlin",
    "wombat",
    "penguin",
    "llama",
    "ferret",
    "badger",
    "axon",
    "synapse",
    "gadget",
    "widget",
    "dongle",
    "blob",
    "thunk",
    "closure",
    "future",
    "promise",
    "channel",
    "mailbox",
    "reactor",
    "scheduler",
];

/// Generate `adjective_noun`, unique against `taken` (case-sensitive).
pub fn generate_pane_name(taken: &HashSet<String>) -> String {
    // Prefer pure combo; fall back to suffix if unlucky.
    for _ in 0..64 {
        let adj = ADJECTIVES[rand_index(ADJECTIVES.len())];
        let noun = NOUNS[rand_index(NOUNS.len())];
        // nouns may contain spaces historically — normalize to underscore
        let noun = noun.replace(' ', "");
        let name = format!("{}_{}", adj, noun);
        if !taken.contains(&name) {
            return name;
        }
    }

    let adj = ADJECTIVES[rand_index(ADJECTIVES.len())];
    let noun = NOUNS[rand_index(NOUNS.len())].replace(' ', "");
    format!("{}_{}_{}", adj, noun, short_suffix())
}

/// Generate a name not used by any pane in `existing_names`.
pub fn generate_unique_pane_name<'a>(existing_names: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let taken: HashSet<String> = existing_names
        .into_iter()
        .flatten()
        .map(|s| s.to_string())
        .collect();
    generate_pane_name(&taken)
}

/// Unique workspace name against existing workspace labels.
pub fn generate_unique_workspace_name<'a>(
    existing_names: impl IntoIterator<Item = &'a str>,
) -> String {
    let taken: HashSet<String> = existing_names.into_iter().map(|s| s.to_string()).collect();
    generate_pane_name(&taken)
}

/// Trim + squash spaces. Reject empty / `/` (address separator).
pub fn sanitize_workspace_name(raw: &str) -> Option<String> {
    let s = raw
        .trim()
        .replace('/', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn rand_index(len: usize) -> usize {
    // Cheap non-crypto RNG from ulid entropy — no extra crate.
    let n = u128::from(ulid::Ulid::new());
    (n as usize) % len.max(1)
}

fn short_suffix() -> String {
    let id = ulid::Ulid::new().to_string();
    id.chars().rev().take(4).collect::<String>().chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_adjective_noun() {
        let name = generate_pane_name(&HashSet::new());
        let parts: Vec<_> = name.split('_').collect();
        assert!(parts.len() >= 2, "got {}", name);
        assert!(ADJECTIVES.contains(&parts[0]), "adj {}", parts[0]);
    }

    #[test]
    fn avoids_collisions() {
        let mut taken = HashSet::new();
        for _ in 0..40 {
            let name = generate_pane_name(&taken);
            assert!(!taken.contains(&name), "duplicate {}", name);
            taken.insert(name);
        }
    }

    #[test]
    fn sanitize_workspace_name_rules() {
        assert_eq!(sanitize_workspace_name("  backend  ").as_deref(), Some("backend"));
        assert_eq!(sanitize_workspace_name("a / b").as_deref(), Some("a_b"));
        assert!(sanitize_workspace_name("   ").is_none());
    }
}
