# Magpie — TODO / backlog

Running list of ideas and noticed issues. The big redesign work is tracked phase
by phase in `docs/superpowers/plans/2026-09-24-magpie-graphite-redesign.md`; this
file is for user-reported issues and cross-cutting ideas.

## Ideas

- [ ] **User-addable themes via standalone JSON files.** Move the theme palettes
  out of the hardcoded Slint array + `themes.rs` registry into a folder of small
  JSON files (e.g. `assets/themes/*.json` for the bundled defaults + a user dir
  like `<data_dir>/themes/*.json`). Each file = `{ id, name, dark, tokens: {…} }`.
  Feasible: make `Theme.palettes` an `in property <[Palette]>` fed from Rust
  instead of a compile-time literal; Rust loads + parses the JSON files at startup
  and pushes the list (bundled first, then user overrides). Then adding a theme
  needs no recompile. (Requested 2026-09-24.)

## Noticed issues (2026-09-24)

- [ ] **⌘K actions palette icons are misaligned.** The per-row action glyphs don't
  line up. (Addressed in Phase 3 — Actions palette per `03-clipboard-actions.png`
  / `Actions.html`.)
- [ ] **Markdown preview not applied for todos and notes.** Note/todo bodies render
  as raw text, not formatted Markdown. Need a Markdown renderer for the notes
  editor preview + task text (headings, bold/italic, lists, code, links).
- [ ] **Inline LaTeX / Typst in Markdown notes & todos.** Support `$…$`/`$$…$$`
  (and/or Typst math) rendered inline within notes and task lines. Ties into the
  math-typesetting spike (plan Phase 8 — gate on binary-size impact first).
- [ ] **Links aren't highlighted on hover.** URLs/links (clipboard link rows,
  `[[wiki-links]]`, bookmark domains) should show a hover affordance — accent
  color and/or underline on hover. (Clipboard/link rows: Phase 3 per `Link.html`;
  wiki-links in notes: notes rendering.)
