# Documentation

How to write the Fini documents. This file links to the rules, and does not
repeat them.

## Where the rules are

- [`naming.md`](naming.md): the English that Fini uses (STE), the check of
  each word, and how to write a name.
- [`glossary.md`](glossary.md): what each word means.
- The `fini-documentation` skill
  ([`SKILL.md`](../.agents/skills/fini-documentation/SKILL.md)): where each
  kind of document goes, how to write it, and which changes need approval.
- The `fini-dev` skill, "Use STE"
  ([`SKILL.md`](../.agents/skills/fini-dev/SKILL.md)): the STE check of each
  diff before you commit.

## Rules

- Write a rule in general words. Do not name the incident that caused it.
- Do not add a condition that the rule does not need.
- Keep facts that code or config already holds in that code or config. The
  document links to it.
- Base each rule on a source or on a decision that was made. Do not invent
  rules.
- Put a rule in the document that owns its topic: development rules go in
  the development guidelines, not in a skill.

## Examples

- Add an example only when the reader cannot understand the rule from the
  text of the rule.
- Do not add an example that the reader does not need. It makes the document
  longer and gives no new information.
