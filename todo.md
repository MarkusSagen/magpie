# Magpie — TODO / backlog

Running list of ideas and noticed issues. The big redesign work is tracked phase
by phase in `docs/superpowers/plans/2026-09-24-magpie-graphite-redesign.md`; this
file is for user-reported issues and cross-cutting ideas.

## Selection actions and cleaners — 2026-11-19

Design: [selection workbench prototype](docs/design/selection-workbench.html).
Implementation is on `selection-actions`; "implemented" is not a native verification claim.

The native workbench now uses the three-column design, with searchable commands
and categories, an editable pipeline, stacked Before/After previews, step examples,
and saved cleaners. Selected-stage results can be copied. Replacement uses the
final result and revalidates the original selection.
Design reference: [refined design 01](docs/design/selection-workbench.html). Three
columns remain the base; icons, short labels, command search/category filters,
move/replace/remove controls, context-aware shortcuts and a Keys sheet are working
in the prototype. Replacement keeps the step position and identity; Undo/Redo
cover pipeline edits. Focus now marks the active column and field, and dims when
the window is inactive. Fields and dropdowns share consistent sizing. Each step
has More then Remove at its top right; the menu keeps Replace, Duplicate,
Disable/Enable, and Example. Optional examples (info icon or F1) show all 16
commands without changing the working text; connected examples use current
settings. Browser checks cover focus, menus, editing, examples and narrow layouts.
HTML/XML cleanup is one **Strip tags** action: extract text, with no markup
formatting choices. Saved prototype XML actions map to it. Header controls now
render once per step, fixing duplicated trash buttons during typing; regression
checks cover repeated keystrokes, deletion, Undo, duplication and tag extraction.

Compare the [five interactive layouts](docs/design/selection-layouts.html), or revisit
the [first HTML prototype](docs/design/selection-actions-prototype.html), for the earlier design alternatives. The five layouts keep Before/After stacked, renumber
selected actions automatically, and open configuration on click. New prototype
actions include Clean, HTML/URL/XML cleanup, Surround, and Expand (escapes, tabs,
abbreviations). Browser checks cover all five layouts, configuration, reordering,
preview stages and these new actions. Commands add pipeline steps; clicking an
existing step opens its settings. The first prototype also explores the plain-text
selection bar. Both prototypes edit examples only. Browser checks cover live
previews, workflow save/load, Replace/Undo, keyboard navigation and responsive
layouts; this does not certify native app behavior.

| Capability | macOS | Windows | Linux | Verification / next step |
|---|---|---|---|---|
| Shared action catalog and typed pipelines | Implemented | Implemented | Implemented | Unit tests; run CI on other OSes |
| Saved cleaners (edit/reorder/disable, encrypted persistence) | Implemented | Implemented | Implemented | Encrypted reopen test; native save, load and settings edit verified |
| Searchable palette and bounded previews | Implemented | Implemented | Implemented | Native workbench verification tracked below |
| Automatic bar and contextual shortcuts | Implemented | Planned | Planned | TextEdit click and contextual shortcut verified; Apple Notes contextual shortcut verified |
| Selection identity and guarded plain-text replacement | Implemented | Planned | Planned | TextEdit and Apple Notes workbench Replace and native Undo verified |
| Per-app enablement, master toggle, bar/shortcut configuration | Implemented | Bar unavailable | Bar unavailable | Config validation tests |
| Global selection-bar summon shortcut | Planned | Planned | Planned | After automatic bar is proven |
| Internal note/scratch selection-only editing | Planned | Planned | Planned | Needs editor selection API; complete text uses shared engine |
| Advanced SQL/encoding option controls | Existing clipboard Transform retained | Retained | Retained | Open All text actions & saved cleaners for pipelines |
| QR generation / QR image output | Existing placeholder | Existing placeholder | Existing placeholder | Current legacy QR transform is not implemented |

### App compatibility — verify explicitly

| App/context | Implementation | Capture/bar | Replace/Undo | Contextual keys / clipboard preservation |
|---|---|---|---|---|
| TextEdit, plain text | MVP target | Bar visually verified | Quick Sort and workbench Replace with native Undo verified | Split click and shortcut verified; text and multi-item RTF clipboard restored |
| Apple Notes, editable body | MVP target | Selection and Split preview verified | Split via Replace and native Undo verified | Contextual Split and newer clipboard copy verified |
| Safari/Chrome, editable fields and read-only pages | Next phase | Not verified | Not verified | Not verified |
| VS Code / Slack / TextMate | Later phase | Not verified | Not verified | Not verified |
| Ghostty / Neovim / terminals | Separate later adapters | Not implemented | Not implemented | Not implemented |
| Secure fields | Explicitly excluded | No bar intended | No replacement | Verify exclusion |

### Remaining native acceptance

- [x] TextEdit selection → bar → quick Sort → native Undo; previous plain-text clipboard restored.
- [x] Fixed first-window collapse reported on Split/Transform; full editor and Split preview inspected at 900 × 660 client size.
- [x] TextEdit selection → Split → load and edit a saved cleaner → Replace → native Undo.
- [x] TextEdit Replace restored plain text, RTF, and a second pasteboard item byte for byte.
- [x] Apple Notes selection → contextual Split → Replace → native Undo in a local note.
- [x] A newer clipboard copy survived replacement. Isolated history captured the newer copy and omitted the replacement text.
- [x] Reopened an encrypted cleaner after app restart and verified the selected step's input and output.
- [x] Replacement declined after the active app changed.
- [x] A LaunchServices run without Accessibility kept the automatic bar off and opened the shared workbench.
- [ ] Permission revocation, source closed or selection edited.
- [ ] Multi-display placement, screen edges, Retina scale, shortcut registration conflicts.
- [ ] Browser adapters and tests after writing-app acceptance; developer apps next; terminals last.
- [ ] Windows native selection backend; Linux X11/Wayland capability assessment and adapters.
- [ ] Recipe sharing/import and executable extensions are deferred, not implemented.

### Other inspiration — deliberately separate follow-ups

- [ ] Day Progress: configurable workday progress, separate from task elapsed time.
- [ ] Selected Supercharge/Vorssaint ideas: OCR, clean URLs, clipboard-to-file.
- [ ] Textream: pinned speaking notes / auto-scroll; evaluate integration before speech-following.
- [ ] Optional integrations: Clop, Handy, MacPacker, Prefab, Forel.
- Disk cleanup (Mole), bootable USB creation (macUSB) and hardware gestures (MacTap) stay outside Magpie.

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
