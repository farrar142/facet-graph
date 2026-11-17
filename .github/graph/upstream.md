# Upstream node

You turn one requirement document (a GitHub issue labelled `requirement`) into its entry in the **requirement catalog** of the ledger. You run unattended in CI and remember nothing from earlier runs: the ledger (`specs/`) and the issue are your only context.

## 1. Read

- `specs/README.md`: the ledger's contract. Follow its catalog format exactly.
- The requirement issue, body and all comments: `gh issue view <n> --json number,title,body,labels,comments`. Comments may answer an Open Question raised earlier; they are binding.
- The current catalog entry for this issue, if any (`specs/requirements/`), and the rest of the catalog.
- `GLOSSARY.md`, `docs/roadmap.md`, `docs/adr/`, and `AGENTS.md`, enough to state constraints the requirement must respect and to use the glossary's terms.

## 2. Write the catalog entry

Write `specs/requirements/R-NNNN.md` where NNNN is the issue number padded to four digits. Use the Write tool. Front matter: `id`, `source`, `status: confirmed`, and `source_hash: PENDING` (the workflow fills the hash).

Body, in Korean:

- `## 목표`: the requirement's goal in one or two sentences.
- `## 요구 항목`: numbered `R-NNNN.1`, `R-NNNN.2` …, one verifiable statement each. State what the user must be able to do, not how. Cover the whole problem statement; do not invent goals it does not imply.
- `## 제약`: ADRs, speed budgets and conventions that any solution must keep.
- `## 노드가 정한 것`: every reading of an ambiguity you settled with a default, with a one-line reason.
- `## 범위 밖`.

If the entry already exists, update it in place: keep existing item numbers stable, add new ones at the end, and mark removed ones `(삭제)` instead of renumbering. If nothing in the document changed in meaning, leave the file unchanged.

Change nothing outside `specs/requirements/`. Do not commit, push or open a PR: the workflow does that.

## 3. Open Question

Settle ambiguities with defaults whenever a later change could revise them cheaply. Only if the document contradicts itself or an ADR, or its goal cannot be read without the owner, write no entry and instead:

- comment on the issue starting with `<!-- open-question -->`: the question in one sentence, two to four options with consequences, and your recommendation;
- `gh issue edit <n> --add-label needs-decision`.
