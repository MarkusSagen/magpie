//! Pure line-level 3-way merge for project file sync (Phase 18b).
//!
//! Unlike [`crate::vault`] (which reconciles whole note *bodies* and keeps-both on
//! conflict), project files must merge at *line* granularity: Magpie only ever
//! rewrites the task lines it owns, so a toggle on line 3 must not disturb line 8,
//! the file's headings, comments, front-matter, or its exact line endings.
//!
//! [`merge3`] takes three versions of a file — `base` (the last agreed content,
//! stored as a per-file baseline), `mine` (`base` with Magpie's line patches
//! applied), and `theirs` (the current disk bytes) — and produces a byte-preserving
//! merge plus a list of conflicting regions. Non-overlapping edits merge silently;
//! a region changed on both sides (differently) is a conflict. In the returned
//! `merged`, conflict regions default to `theirs` (the "File wins" resolution); the
//! caller decides whether to apply `merged` (File wins) or hold it and surface the
//! conflicts for Review (Ask).
//!
//! Everything here is pure and unit-tested with string inputs; the DB/FS
//! orchestration (`reconcile_project_file`, the watcher) is a thin layer on top,
//! mirroring how [`crate::vault::resolve_action`] sits under `reconcile_vault`.

/// One conflicting region: the three divergent slices (each already joined with its
/// original line endings) and the 1-based line where the region begins in `merged`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub base: String,
    pub mine: String,
    pub theirs: String,
    /// 1-based line number in `merged` where this conflict region starts (for the
    /// "Line N changed in Magpie and on disk" banner).
    pub line: usize,
}

/// The result of a 3-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge3 {
    /// Auto-merged content. Non-conflicting regions are merged; each conflict region
    /// defaults to `theirs` (the File-wins resolution).
    pub merged: String,
    /// Conflicting regions, in order. Empty ⇒ a fully clean, silent merge.
    pub conflicts: Vec<Conflict>,
}

impl Merge3 {
    /// A fully clean merge (no overlapping edits).
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Split `s` into lines, each INCLUDING its trailing `\n` (and the `\r` of a `\r\n`),
/// so that `parts.concat() == s` byte-for-byte. A final line without a newline is
/// returned without one. An empty string yields no parts.
fn split_keep_eol(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'\n' {
            out.push(&s[start..=i]);
            start = i + 1;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Longest common subsequence of two line arrays, as increasing matched index pairs
/// `(a_idx, b_idx)`. Lines are compared by exact bytes (EOL included), so an EOL-only
/// change counts as a real change. O(n·m) — fine for task files (small); the
/// read-only code-comment lens never merges, so large files don't hit this.
fn lcs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    // dp[i][j] = LCS length of a[i..] and b[j..].
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut res = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            res.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    res
}

/// One side's diff against base, as a replacement of `base[b0..b1)` with `repl`. A
/// pure insertion has `b0 == b1` (a zero-width edit at that gap); a deletion has an
/// empty `repl`.
struct Hunk<'a> {
    b0: usize,
    b1: usize,
    repl: Vec<&'a str>,
}

/// Turn `matches` (LCS pairs of unchanged lines) into the list of changed hunks that
/// take `base` to `other`. Consecutive changes are already coalesced because a run of
/// non-matching lines between two matches becomes a single hunk.
fn diff_hunks<'a>(
    base: &[&'a str],
    other: &[&'a str],
    matches: &[(usize, usize)],
) -> Vec<Hunk<'a>> {
    let mut hs = Vec::new();
    let (mut pb, mut po) = (0usize, 0usize);
    for &(mb, mo) in matches {
        if mb > pb || mo > po {
            hs.push(Hunk {
                b0: pb,
                b1: mb,
                repl: other[po..mo].to_vec(),
            });
        }
        pb = mb + 1;
        po = mo + 1;
    }
    if pb < base.len() || po < other.len() {
        hs.push(Hunk {
            b0: pb,
            b1: base.len(),
            repl: other[po..].to_vec(),
        });
    }
    hs
}

/// Reconstruct one side's content over `base[lo..hi)` by applying its `hunks` (sorted,
/// all within `[lo, hi)`), preserving the untouched base lines between them.
fn build_region(base: &[&str], lo: usize, hi: usize, hunks: &[&Hunk]) -> String {
    let mut s = String::new();
    let mut pos = lo;
    for h in hunks {
        for l in &base[pos..h.b0] {
            s.push_str(l);
        }
        for l in &h.repl {
            s.push_str(l);
        }
        pos = h.b1;
    }
    for l in &base[pos..hi] {
        s.push_str(l);
    }
    s
}

/// 3-way merge of `base`, `mine`, and `theirs` at line granularity (see module docs).
///
/// Works from the two per-side hunk lists (`base→mine`, `base→theirs`). Insertions are
/// attributed to gaps, so a disk insertion adjacent to a Magpie edit merges silently;
/// a conflict is raised only when the two sides' changed base ranges actually overlap
/// (or both insert different content at the same gap). Conflict regions in `merged`
/// take `theirs` (the File-wins / "keep disk" resolution).
pub fn merge3(base: &str, mine: &str, theirs: &str) -> Merge3 {
    merge3_with(base, mine, theirs, false)
}

/// The merged content with every conflict region resolved to one side: `prefer_mine`
/// picks Magpie's version ("Keep mine" in Review), otherwise disk's ("Keep disk").
/// Non-conflicting regions merge as usual.
pub fn resolve(base: &str, mine: &str, theirs: &str, prefer_mine: bool) -> String {
    merge3_with(base, mine, theirs, prefer_mine).merged
}

fn merge3_with(base: &str, mine: &str, theirs: &str, prefer_mine: bool) -> Merge3 {
    let b = split_keep_eol(base);
    let m = split_keep_eol(mine);
    let t = split_keep_eol(theirs);

    let mh = diff_hunks(&b, &m, &lcs(&b, &m));
    let th = diff_hunks(&b, &t, &lcs(&b, &t));

    let mut out = String::new();
    let mut conflicts = Vec::new();
    let mut out_lines = 0usize;
    let (mut mi, mut ti) = (0usize, 0usize);
    let mut p = 0usize; // base cursor

    // Emit a run of &str lines, counting them.
    macro_rules! emit {
        ($lines:expr) => {{
            for l in $lines {
                out.push_str(l);
                out_lines += 1;
            }
        }};
    }
    // Emit a reconstructed region string, counting its lines exactly.
    macro_rules! emit_str {
        ($s:expr) => {{
            let s: &str = $s;
            out.push_str(s);
            out_lines += split_keep_eol(s).len();
        }};
    }

    loop {
        let mnext = mh.get(mi);
        let tnext = th.get(ti);
        if mnext.is_none() && tnext.is_none() {
            for l in &b[p..] {
                out.push_str(l);
            }
            break;
        }
        let mb0 = mnext.map(|h| h.b0).unwrap_or(usize::MAX);
        let tb0 = tnext.map(|h| h.b0).unwrap_or(usize::MAX);
        let stop = mb0.min(tb0).min(b.len());
        if p < stop {
            emit!(&b[p..stop]);
            p = stop;
        }

        let m_here = mnext.filter(|h| h.b0 <= p);
        let t_here = tnext.filter(|h| h.b0 <= p);

        // Whether a conflict region must be resolved (both sides overlap in base range).
        let overlap = match (m_here, t_here) {
            (Some(hm), None) => tnext.is_some_and(|ht| ht.b0 < hm.b1),
            (None, Some(ht)) => mnext.is_some_and(|hm| hm.b0 < ht.b1),
            (Some(hm), Some(ht)) => !(hm.b1 == hm.b0 || ht.b1 == ht.b0), // both real replacements
            (None, None) => unreachable!("stop guarantees a hunk at p"),
        };

        if overlap {
            // Group all mutually-overlapping hunks from both sides into one region.
            let lo = p;
            let mut hi = p;
            let mut mreg: Vec<&Hunk> = Vec::new();
            let mut treg: Vec<&Hunk> = Vec::new();
            // Seed with the hunk(s) that start at lo.
            if let Some(h) = mh.get(mi) {
                if h.b0 == lo {
                    hi = hi.max(h.b1);
                    mreg.push(h);
                    mi += 1;
                }
            }
            if let Some(h) = th.get(ti) {
                if h.b0 == lo {
                    hi = hi.max(h.b1);
                    treg.push(h);
                    ti += 1;
                }
            }
            // Chain in any further hunk strictly inside the growing region.
            loop {
                let mut grew = false;
                while let Some(h) = mh.get(mi) {
                    if h.b0 < hi {
                        hi = hi.max(h.b1);
                        mreg.push(h);
                        mi += 1;
                        grew = true;
                    } else {
                        break;
                    }
                }
                while let Some(h) = th.get(ti) {
                    if h.b0 < hi {
                        hi = hi.max(h.b1);
                        treg.push(h);
                        ti += 1;
                        grew = true;
                    } else {
                        break;
                    }
                }
                if !grew {
                    break;
                }
            }
            let base_r = b[lo..hi].concat();
            let mine_r = build_region(&b, lo, hi, &mreg);
            let theirs_r = build_region(&b, lo, hi, &treg);
            if mine_r == base_r {
                // Only the disk side changed within this region.
                emit_str!(&theirs_r);
            } else if theirs_r == base_r || mine_r == theirs_r {
                // Only Magpie changed, or both made the identical change.
                emit_str!(&mine_r);
            } else {
                conflicts.push(Conflict {
                    base: base_r,
                    mine: mine_r.clone(),
                    theirs: theirs_r.clone(),
                    line: out_lines + 1,
                });
                if prefer_mine {
                    emit_str!(&mine_r);
                } else {
                    emit_str!(&theirs_r);
                }
            }
            p = hi;
            continue;
        }

        match (m_here, t_here) {
            (Some(hm), None) => {
                emit!(&hm.repl);
                p = hm.b1;
                mi += 1;
            }
            (None, Some(ht)) => {
                emit!(&ht.repl);
                p = ht.b1;
                ti += 1;
            }
            (Some(hm), Some(ht)) => {
                let m_ins = hm.b1 == hm.b0;
                let t_ins = ht.b1 == ht.b0;
                if m_ins && t_ins {
                    // Both insert at the same gap.
                    if hm.repl == ht.repl {
                        emit!(&hm.repl);
                    } else {
                        conflicts.push(Conflict {
                            base: String::new(),
                            mine: hm.repl.concat(),
                            theirs: ht.repl.concat(),
                            line: out_lines + 1,
                        });
                        if prefer_mine {
                            emit!(&hm.repl);
                        } else {
                            emit!(&ht.repl);
                        }
                    }
                    mi += 1;
                    ti += 1;
                } else if m_ins {
                    // Mine inserts before theirs' replacement at this line — insert first.
                    emit!(&hm.repl);
                    mi += 1;
                } else {
                    emit!(&ht.repl);
                    ti += 1;
                }
            }
            (None, None) => unreachable!(),
        }
    }

    Merge3 {
        merged: out,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(base: &str, mine: &str, theirs: &str, expect: &str) -> Merge3 {
        let r = merge3(base, mine, theirs);
        assert!(
            r.is_clean(),
            "expected a clean merge, got conflicts: {:?}",
            r.conflicts
        );
        assert_eq!(r.merged, expect);
        r
    }

    // ---- split/round-trip ----

    #[test]
    fn split_keep_eol_roundtrips_all_shapes() {
        for s in [
            "", "a", "a\n", "a\nb", "a\nb\n", "a\r\nb\n", "\n\n", "a\n\nb",
        ] {
            assert_eq!(split_keep_eol(s).concat(), s, "roundtrip {s:?}");
        }
    }

    // ---- A. idempotence / no-op ----

    #[test]
    fn a1_no_change_is_identity() {
        let s = "# T\n- [ ] one\n- [ ] two\n";
        clean(s, s, s, s);
        assert!(merge3(s, s, s).conflicts.is_empty());
    }

    // ---- B. one-sided edits ----

    #[test]
    fn b1_disk_toggle_pulls() {
        let base = "# T\n- [ ] one\n- [ ] two\n";
        let disk = "# T\n- [x] one\n- [ ] two\n";
        clean(base, base, disk, disk); // mine == base, take theirs
    }

    #[test]
    fn b2_magpie_toggle_pushes() {
        let base = "# T\n- [ ] one\n- [ ] two\n";
        let mine = "# T\n- [ ] one\n- [x] two\n";
        clean(base, mine, base, mine); // theirs == base, take mine
    }

    #[test]
    fn b5_disk_edits_non_task_line() {
        let base = "# Title\n- [ ] one\n";
        let disk = "# Renamed title\n- [ ] one\n";
        clean(base, base, disk, disk);
    }

    // ---- C. non-overlapping concurrent edits merge silently ----

    #[test]
    fn c1_both_change_different_lines() {
        let base = "- [ ] a\n- [ ] b\n- [ ] c\n";
        let mine = "- [x] a\n- [ ] b\n- [ ] c\n"; // toggled line 0
        let disk = "- [ ] a\n- [ ] b\n- [ ] c changed\n"; // edited line 2
        clean(base, mine, disk, "- [x] a\n- [ ] b\n- [ ] c changed\n");
    }

    #[test]
    fn c2_magpie_appends_disk_edits_other() {
        let base = "- [ ] a\n- [ ] b\n";
        let mine = "- [ ] a\n- [ ] b\n- [ ] new\n"; // appended
        let disk = "- [x] a\n- [ ] b\n"; // toggled line 0
        clean(base, mine, disk, "- [x] a\n- [ ] b\n- [ ] new\n");
    }

    #[test]
    fn c3_disk_adds_new_task_magpie_toggles() {
        let base = "- [ ] a\n- [ ] b\n";
        let mine = "- [x] a\n- [ ] b\n"; // toggled line 0
        let disk = "- [ ] a\n- [ ] b\n- [ ] fromdisk\n"; // appended on disk
        clean(base, mine, disk, "- [x] a\n- [ ] b\n- [ ] fromdisk\n");
    }

    // ---- D. overlapping edits conflict ----

    #[test]
    fn d1_same_line_both_sides_conflicts() {
        let base = "- [ ] a\n- [ ] task\n- [ ] c\n";
        let mine = "- [ ] a\n- [x] task\n- [ ] c\n"; // Magpie toggled
        let disk = "- [ ] a\n- [ ] task edited\n- [ ] c\n"; // disk edited text
        let r = merge3(base, mine, disk);
        assert_eq!(r.conflicts.len(), 1, "one conflict, got {:?}", r.conflicts);
        let c = &r.conflicts[0];
        assert_eq!(c.base, "- [ ] task\n");
        assert_eq!(c.mine, "- [x] task\n");
        assert_eq!(c.theirs, "- [ ] task edited\n");
        assert_eq!(c.line, 2, "1-based conflict line");
        // merged defaults conflict region to theirs (File-wins).
        assert_eq!(r.merged, disk);
    }

    #[test]
    fn d3_conflict_is_single_not_double() {
        let base = "x\ntask\ny\n";
        let mine = "x\nMINE\ny\n";
        let disk = "x\nTHEIRS\ny\n";
        let r = merge3(base, mine, disk);
        assert_eq!(r.conflicts.len(), 1);
    }

    #[test]
    fn d7_delete_vs_edit_conflicts() {
        let base = "a\ntask\nb\n";
        let mine = "a\nb\n"; // Magpie deleted the task line
        let disk = "a\ntask edited\nb\n"; // disk edited it
        let r = merge3(base, mine, disk);
        assert_eq!(r.conflicts.len(), 1, "delete vs edit must conflict");
    }

    #[test]
    fn delete_on_one_side_only_is_silent() {
        let base = "a\ntask\nb\n";
        let mine = "a\nb\n"; // Magpie deleted
        clean(base, mine, base, "a\nb\n"); // disk unchanged → take the deletion
    }

    // ---- E. structural ----

    #[test]
    fn e2_disk_inserts_line_above_no_spurious_conflict() {
        let base = "- [ ] a\n- [ ] b\n";
        let mine = "- [x] a\n- [ ] b\n"; // toggled a
        let disk = "intro\n- [ ] a\n- [ ] b\n"; // inserted a line at top
        clean(base, mine, disk, "intro\n- [x] a\n- [ ] b\n");
    }

    #[test]
    fn e5_both_append_different_tasks_kept() {
        let base = "- [ ] a\n";
        let mine = "- [ ] a\n- [ ] mine\n";
        let disk = "- [ ] a\n- [ ] theirs\n";
        // Both appended after the same anchor → divergent tail → conflict (not lost).
        let r = merge3(base, mine, disk);
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].mine, "- [ ] mine\n");
        assert_eq!(r.conflicts[0].theirs, "- [ ] theirs\n");
    }

    // ---- F. byte preservation ----

    #[test]
    fn f1_crlf_only_toggled_line_changes() {
        let base = "# T\r\n- [ ] one\r\n- [ ] two\r\n";
        let mine = "# T\r\n- [x] one\r\n- [ ] two\r\n";
        clean(base, mine, base, mine);
        // Every EOL stays CRLF.
        assert!(!merge3(base, mine, base).merged.contains("\n\n"));
        assert_eq!(merge3(base, mine, base).merged.matches("\r\n").count(), 3);
    }

    #[test]
    fn f2_mixed_eol_untouched_lines_keep_their_ending() {
        let base = "a\r\nb\nc\r\n";
        let disk = "a\r\nB\nc\r\n"; // edited middle (LF) line only
        let r = clean(base, base, disk, disk);
        assert_eq!(r.merged, "a\r\nB\nc\r\n");
    }

    #[test]
    fn f3_no_final_newline_preserved() {
        let base = "- [ ] one\n- [ ] two"; // no trailing newline
        let mine = "- [ ] one\n- [x] two";
        clean(base, mine, base, mine);
        assert!(!merge3(base, mine, base).merged.ends_with('\n'));
    }

    #[test]
    fn f4_indentation_preserved() {
        let base = "- [ ] a\n\t- [ ] nested\n";
        let mine = "- [ ] a\n\t- [x] nested\n";
        clean(base, mine, base, mine);
    }

    #[test]
    fn f5_front_matter_never_rewritten_by_task_patch() {
        let base = "---\ntitle: X\n---\n- [ ] a\n";
        let mine = "---\ntitle: X\n---\n- [x] a\n";
        let disk = base;
        let r = clean(base, mine, disk, mine);
        assert!(r.merged.starts_with("---\ntitle: X\n---\n"));
    }

    // ---- H. formats (line-level; the toggling primitive is format-specific but the
    //         merge is format-agnostic — here we prove org/todo.txt lines merge) ----

    #[test]
    fn h2_org_toggle_line_merges() {
        let base = "* TODO write tests\n  SCHEDULED: <2026-09-27>\n";
        let mine = "* DONE write tests\n  SCHEDULED: <2026-09-27>\n";
        clean(base, mine, base, mine); // SCHEDULED line untouched
    }

    #[test]
    fn h4_todotxt_completion_prefix_merges() {
        let base = "(A) ship +magpie @home\n";
        let mine = "x 2026-09-26 (A) ship +magpie @home\n";
        clean(base, mine, base, mine);
    }

    // ---- empty-file edges ----

    #[test]
    fn empty_base_both_add_same_is_clean() {
        clean("", "- [ ] a\n", "- [ ] a\n", "- [ ] a\n");
    }

    #[test]
    fn empty_base_both_add_different_conflicts() {
        let r = merge3("", "- [ ] mine\n", "- [ ] theirs\n");
        assert_eq!(r.conflicts.len(), 1);
    }

    // ---- resolve (Review: keep mine / keep disk) ----

    #[test]
    fn resolve_picks_the_chosen_side_for_conflicts() {
        let base = "- [ ] a\n- [ ] task\n- [ ] c\n";
        let mine = "- [ ] a\n- [x] task\n- [ ] c\n";
        let disk = "- [ ] a\n- [ ] task edited\n- [ ] c\n";
        // Keep disk == the default merged.
        assert_eq!(resolve(base, mine, disk, false), disk);
        assert_eq!(
            resolve(base, mine, disk, false),
            merge3(base, mine, disk).merged
        );
        // Keep mine substitutes Magpie's line, preserving the rest.
        assert_eq!(
            resolve(base, mine, disk, true),
            "- [ ] a\n- [x] task\n- [ ] c\n"
        );
    }

    #[test]
    fn resolve_keeps_disjoint_edits_but_takes_whole_conflict_region() {
        // Magpie toggled line 0 (disjoint) and line 1; disk edited lines 1 and 2, so
        // the b/c edits chain with mine's b edit into one conflict region.
        let base = "- [ ] a\n- [ ] b\n- [ ] c\n";
        let mine = "- [x] a\n- [x] b\n- [ ] c\n";
        let disk = "- [ ] a\n- [ ] b renamed\n- [ ] c done?\n";
        // mine's contiguous a+b edit is one hunk that overlaps disk's b+c edit, so the
        // whole [0..3) is one conflict region taken wholesale from the chosen side —
        // you can't cherry-pick within a hunk.
        assert_eq!(
            resolve(base, mine, disk, true),
            "- [x] a\n- [x] b\n- [ ] c\n"
        );
        assert_eq!(
            resolve(base, mine, disk, false),
            "- [ ] a\n- [ ] b renamed\n- [ ] c done?\n"
        );
    }
}
