//! Checklist storage codec for issue #128 (Track A — Markdown task-list, chosen after the
//! spike documented in `space_sync` history / issue #128 / fini-wiki, then refined so the
//! checklist reuses the existing `description` field instead of a dedicated column). A checklist
//! quest's `description` holds GitHub-style task-list markdown (`- [ ] text <!--k=id-->`) instead
//! of prose — the same textarea, just parsed/rendered differently when `quests.is_checklist` is
//! set. `quest_series.description` holds the recurring template in the same format, gated by
//! `quest_series.is_checklist`. `quests.checklist_base` is separate, device-local bookkeeping for
//! the sync merge below — never the checklist content itself.
//!
//! The embedded `<!--k=id-->` token gives each line stable identity across edits, which is what
//! makes the per-item sync merge (`merge_3way`, used by `space_sync::commands::apply_sync_event`)
//! and the recurrence scope reconciliation (`reconcile_future_scope`) possible.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
}

pub fn new_item_id() -> String {
    Uuid::new_v4().to_string()
}

/// Parses task-list lines (`- [ ] text` / `- [x] text`) with an optional trailing hidden id
/// token. Non-task lines are ignored — a checklist quest's markdown field is expected to be
/// checklist-only (prose stays in the separate `description` field).
pub fn parse(src: &str) -> Vec<ChecklistItem> {
    let mut items = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        let (checked, rest) = if let Some(rest) = line.strip_prefix("- [ ] ") {
            (false, rest)
        } else if let Some(rest) = line
            .strip_prefix("- [x] ")
            .or_else(|| line.strip_prefix("- [X] "))
        {
            (true, rest)
        } else {
            continue;
        };

        let (text, id) = match rest.rfind("<!--k=") {
            Some(idx) if rest.ends_with("-->") => {
                let id = &rest[idx + 6..rest.len() - 3];
                (rest[..idx].trim_end().to_string(), id.to_string())
            }
            _ => (rest.to_string(), new_item_id()),
        };
        items.push(ChecklistItem { id, text, checked });
    }
    items
}

pub fn serialize(items: &[ChecklistItem]) -> String {
    items
        .iter()
        .map(|it| {
            let box_ = if it.checked { "x" } else { " " };
            format!("- [{box_}] {} <!--k={}-->", it.text, it.id)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn parse_opt(src: Option<&str>) -> Vec<ChecklistItem> {
    src.map(parse).unwrap_or_default()
}

/// `(done, total)` counts, for list/editor badges.
pub fn counts(src: Option<&str>) -> (usize, usize) {
    let items = parse_opt(src);
    let done = items.iter().filter(|it| it.checked).count();
    (done, items.len())
}

/// Recurrence: a fresh occurrence copies the series template with every box reset.
pub fn reset_unchecked(src: &str) -> String {
    let items: Vec<ChecklistItem> = parse(src)
        .into_iter()
        .map(|it| ChecklistItem {
            checked: false,
            ..it
        })
        .collect();
    serialize(&items)
}

pub fn add_item(src: Option<&str>, text: &str) -> String {
    let mut items = parse_opt(src);
    items.push(ChecklistItem {
        id: new_item_id(),
        text: text.to_string(),
        checked: false,
    });
    serialize(&items)
}

pub fn set_checked(src: &str, item_id: &str, checked: bool) -> String {
    let mut items = parse(src);
    if let Some(it) = items.iter_mut().find(|it| it.id == item_id) {
        it.checked = checked;
    }
    serialize(&items)
}

pub fn set_text(src: &str, item_id: &str, text: &str) -> String {
    let mut items = parse(src);
    if let Some(it) = items.iter_mut().find(|it| it.id == item_id) {
        it.text = text.to_string();
    }
    serialize(&items)
}

pub fn remove_item(src: &str, item_id: &str) -> String {
    let items: Vec<ChecklistItem> = parse(src)
        .into_iter()
        .filter(|it| it.id != item_id)
        .collect();
    serialize(&items)
}

/// Reorders items to match `ordered_ids`. Ids not present in `ordered_ids` are appended in their
/// original relative order (defensive — should not happen if the caller passes a full id list).
pub fn reorder(src: &str, ordered_ids: &[String]) -> String {
    let items = parse(src);
    let mut by_id: std::collections::HashMap<String, ChecklistItem> =
        items.into_iter().map(|it| (it.id.clone(), it)).collect();
    let mut result: Vec<ChecklistItem> = Vec::new();
    for id in ordered_ids {
        if let Some(it) = by_id.remove(id) {
            result.push(it);
        }
    }
    let mut leftovers: Vec<ChecklistItem> = by_id.into_values().collect();
    result.append(&mut leftovers);
    serialize(&result)
}

/// "This and future occurrences": the series template changed. Reconcile the *current*
/// occurrence's checklist against the new template — matched ids keep their `checked` state
/// (per #128: "preserve checks on unchanged current-occurrence items"), new template items are
/// added unchecked, and template-removed items are dropped from the occurrence.
pub fn reconcile_future_scope(
    current_occurrence_md: Option<&str>,
    new_template_md: &str,
) -> String {
    let current = parse_opt(current_occurrence_md);
    let checked_by_id: std::collections::HashMap<&str, bool> = current
        .iter()
        .map(|it| (it.id.as_str(), it.checked))
        .collect();

    let reconciled: Vec<ChecklistItem> = parse(new_template_md)
        .into_iter()
        .map(|template_item| {
            let checked = checked_by_id
                .get(template_item.id.as_str())
                .copied()
                .unwrap_or(false);
            ChecklistItem {
                checked,
                ..template_item
            }
        })
        .collect();
    serialize(&reconciled)
}

/// 3-way merge for the per-item sync path (`space_sync::commands::apply_sync_event`). `base` is
/// the device-local `checklist_base` — the last value both sides last agreed on. Returns the
/// merged markdown and whether an irreconcilable same-item conflict was hit (logged, not fatal —
/// see module docs / issue #128 spike write-up for why this is a narrow, accepted deviation from
/// per-item deterministic LWW for same-item *text* edits specifically).
pub fn merge_3way(base: Option<&str>, local: &str, remote: &str) -> (String, bool) {
    use std::collections::{HashMap, HashSet};

    let base_items = parse_opt(base);
    let local_items = parse(local);
    let remote_items = parse(remote);

    let base_by_id: HashMap<&str, &ChecklistItem> =
        base_items.iter().map(|it| (it.id.as_str(), it)).collect();
    let local_by_id: HashMap<&str, &ChecklistItem> =
        local_items.iter().map(|it| (it.id.as_str(), it)).collect();
    let remote_by_id: HashMap<&str, &ChecklistItem> =
        remote_items.iter().map(|it| (it.id.as_str(), it)).collect();

    // Ordering must be deterministic regardless of which side calls merge_3way as "local" vs
    // "remote" (see track_a_/merge_same_item_text_conflict_chooses_same_winner_from_either_side
    // for the same requirement applied to conflict resolution) — otherwise two devices that both
    // independently added items from the same base would each serialize a different item order,
    // and the convergence push-back (which re-emits whenever merged != incoming) would never
    // settle. Base order is unambiguous (both sides start from the same base), so items retained
    // from base keep base's order; anything new to either side is appended in a symmetric,
    // side-independent order (sorted by id).
    let mut order: Vec<&str> = Vec::new();
    let mut seen = HashSet::new();
    for it in base_items.iter() {
        if (local_by_id.contains_key(it.id.as_str()) || remote_by_id.contains_key(it.id.as_str()))
            && seen.insert(it.id.as_str())
        {
            order.push(it.id.as_str());
        }
    }
    let mut new_ids: Vec<&str> = local_items
        .iter()
        .chain(remote_items.iter())
        .map(|it| it.id.as_str())
        .filter(|id| seen.insert(id))
        .collect();
    new_ids.sort_unstable();
    order.extend(new_ids);

    let mut merged = Vec::new();
    let mut had_conflict = false;

    for id in order {
        let l = local_by_id.get(id).copied();
        let r = remote_by_id.get(id).copied();
        let b = base_by_id.get(id).copied();

        let resolved = match (l, r) {
            (Some(l), Some(r)) if l == r => l.clone(),
            (Some(l), Some(r)) => {
                let base_text = b.map(|b| b.text.as_str());
                let text = match base_text {
                    Some(bt) if bt == l.text && bt != r.text => r.text.clone(),
                    Some(bt) if bt == r.text && bt != l.text => l.text.clone(),
                    _ if l.text == r.text => l.text.clone(),
                    _ => {
                        // Genuine same-item text divergence: no per-item updated_at/device_id is
                        // available on a single markdown field, so Fini's deterministic LWW rule
                        // cannot be applied here. Choose the same lexical winner on every peer
                        // instead of keeping whichever side is local, so conflict convergence
                        // events eventually settle on one value.
                        had_conflict = true;
                        if l.text <= r.text {
                            l.text.clone()
                        } else {
                            r.text.clone()
                        }
                    }
                };
                let checked = match b.map(|b| b.checked) {
                    Some(base_checked)
                        if l.checked == base_checked && r.checked != base_checked =>
                    {
                        r.checked
                    }
                    Some(base_checked)
                        if r.checked == base_checked && l.checked != base_checked =>
                    {
                        l.checked
                    }
                    _ if l.checked == r.checked => l.checked,
                    _ => l.checked || r.checked,
                };
                ChecklistItem {
                    id: l.id.clone(),
                    text,
                    checked,
                }
            }
            (Some(l), None) => match b {
                Some(b) if l == b => continue,
                Some(_) => {
                    had_conflict = true;
                    l.clone()
                }
                None => l.clone(),
            },
            (None, Some(r)) => match b {
                Some(b) if r == b => continue,
                Some(_) => {
                    had_conflict = true;
                    r.clone()
                }
                None => r.clone(),
            },
            (None, None) => continue,
        };
        merged.push(resolved);
    }

    (serialize(&merged), had_conflict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let items = vec![
            ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: true,
            },
        ];
        let md = serialize(&items);
        assert_eq!(parse(&md), items);
    }

    #[test]
    fn recurrence_reset_all_unchecked() {
        let template = serialize(&[
            ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: true,
            },
            ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: true,
            },
        ]);
        let occurrence = parse(&reset_unchecked(&template));
        assert!(occurrence.iter().all(|it| !it.checked));
        assert_eq!(occurrence.len(), 2);
    }

    #[test]
    fn add_toggle_edit_remove_round_trip() {
        let md = add_item(None, "headphones");
        let id = parse(&md)[0].id.clone();
        let md = set_checked(&md, &id, true);
        assert!(parse(&md)[0].checked);
        let md = set_text(&md, &id, "over-ear headphones");
        assert_eq!(parse(&md)[0].text, "over-ear headphones");
        let md = remove_item(&md, &id);
        assert!(parse(&md).is_empty());
    }

    #[test]
    fn reorder_matches_requested_order() {
        let md = serialize(&[
            ChecklistItem {
                id: "a1".into(),
                text: "one".into(),
                checked: false,
            },
            ChecklistItem {
                id: "a2".into(),
                text: "two".into(),
                checked: false,
            },
        ]);
        let reordered = parse(&reorder(&md, &["a2".to_string(), "a1".to_string()]));
        assert_eq!(reordered[0].id, "a2");
        assert_eq!(reordered[1].id, "a1");
    }

    #[test]
    fn future_scope_preserves_checked_adds_unchecked_drops_removed() {
        let occurrence = serialize(&[
            ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: true,
            },
            ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        // template drops a2, adds a3
        let new_template = serialize(&[
            ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            ChecklistItem {
                id: "a3".into(),
                text: "lunch".into(),
                checked: false,
            },
        ]);
        let reconciled = parse(&reconcile_future_scope(Some(&occurrence), &new_template));
        assert_eq!(reconciled.len(), 2);
        assert!(
            reconciled.iter().find(|it| it.id == "a1").unwrap().checked,
            "unchanged item keeps its check"
        );
        assert!(
            !reconciled.iter().find(|it| it.id == "a3").unwrap().checked,
            "new item is unchecked"
        );
        assert!(
            reconciled.iter().find(|it| it.id == "a2").is_none(),
            "removed item is dropped"
        );
    }

    #[test]
    fn merge_independently_added_items_converge_on_the_same_order_from_either_side() {
        let base = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let mut local_items = parse(&base);
        local_items.push(ChecklistItem {
            id: "b2".into(),
            text: "key fob".into(),
            checked: false,
        });
        let local = serialize(&local_items);
        let mut remote_items = parse(&base);
        remote_items.push(ChecklistItem {
            id: "c3".into(),
            text: "lunch".into(),
            checked: false,
        });
        let remote = serialize(&remote_items);

        let (from_local_md, _) = merge_3way(Some(&base), &local, &remote);
        let (from_remote_md, _) = merge_3way(Some(&base), &remote, &local);

        assert_eq!(
            from_local_md, from_remote_md,
            "merge order must not depend on which side calls merge_3way as local vs remote, \
             or the convergence push-back never settles"
        );
    }

    #[test]
    fn merge_independent_item_edits_both_survive() {
        let base = serialize(&[
            ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: false,
            },
        ]);
        let mut local_items = parse(&base);
        local_items[0].checked = true; // device A packed headphones
        let local = serialize(&local_items);
        let mut remote_items = parse(&base);
        remote_items[1].checked = true; // device B packed key fob
        let remote = serialize(&remote_items);

        let (merged_md, had_conflict) = merge_3way(Some(&base), &local, &remote);
        let merged = parse(&merged_md);
        assert!(!had_conflict);
        assert!(merged.iter().find(|it| it.id == "a1").unwrap().checked);
        assert!(merged.iter().find(|it| it.id == "a2").unwrap().checked);
    }

    #[test]
    fn merge_one_sided_uncheck_wins_against_unchanged_peer() {
        let base = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: true,
        }]);
        let local = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let remote = base.clone();

        let (merged_md, had_conflict) = merge_3way(Some(&base), &local, &remote);
        let merged = parse(&merged_md);
        assert!(!had_conflict);
        assert!(!merged[0].checked);
    }

    #[test]
    fn merge_one_sided_remove_wins_against_unchanged_peer() {
        let base = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let local = String::new();
        let remote = base.clone();

        let (merged_md, had_conflict) = merge_3way(Some(&base), &local, &remote);
        assert!(!had_conflict);
        assert!(parse(&merged_md).is_empty());
    }

    #[test]
    fn merge_same_item_text_conflict_is_flagged() {
        let base = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let local = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones (blue)".into(),
            checked: false,
        }]);
        let remote = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "over-ear headphones".into(),
            checked: false,
        }]);

        let (_merged_md, had_conflict) = merge_3way(Some(&base), &local, &remote);
        assert!(had_conflict);
    }

    #[test]
    fn merge_same_item_text_conflict_chooses_same_winner_from_either_side() {
        let base = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let local = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "headphones (blue)".into(),
            checked: false,
        }]);
        let remote = serialize(&[ChecklistItem {
            id: "a1".into(),
            text: "over-ear headphones".into(),
            checked: false,
        }]);

        let (local_merged_md, local_had_conflict) = merge_3way(Some(&base), &local, &remote);
        let (remote_merged_md, remote_had_conflict) = merge_3way(Some(&base), &remote, &local);
        let local_merged = parse(&local_merged_md);
        let remote_merged = parse(&remote_merged_md);

        assert!(local_had_conflict);
        assert!(remote_had_conflict);
        assert_eq!(local_merged, remote_merged);
        assert_eq!(local_merged[0].text, "headphones (blue)");
    }
}
