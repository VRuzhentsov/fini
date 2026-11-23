---
name: fini-documentation
description: "How the agent changes Fini documentation: read the doc rules (where each kind of doc lives in docs/, the STE writing rules, one place per topic, ASCII trees for architecture, mermaid only for complex algorithms), get approval for new doc files, and check the links. Use before adding, moving, or changing any doc, spec, README, ADR, glossary entry, or architecture page in the Fini repo."
---

# Fini documentation

Use this skill before you add, move or change documentation in this repo. The
rules are in `docs/documentation.md`; the STE rules are in `docs/naming.md`.

1. Read `docs/documentation.md` and `docs/naming.md`.
2. Find the one document that owns the topic ("Where documents go"). Change
   that document, and link to it from the others.
3. When you change a README or a companion spec next to the code, move its
   content to the right place in `docs/`, delete it from the code tree, and
   update the links to it.
4. Before you make a new document file or add a glossary term, get the
   approval of the project owner ("Approval"). Change an ADR only while you
   discuss a plan with the project owner.
5. Check:
   - Every link in the changed documents goes to a file that exists.
   - No document repeats a topic that has its own place.
