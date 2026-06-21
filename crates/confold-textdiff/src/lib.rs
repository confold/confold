//! Line- and word-level text diff producing an aligned, side-by-side model for the GUI.
//!
//! [`diff_text`] returns a [`FileDiff`]: a flat list of aligned [`DiffRow`]s (Equal / Insert / Delete /
//! Replace). Replaced rows carry word-level change ranges (character offsets) for intra-line highlighting.
//! Built on the `similar` crate (Myers/patience); no bespoke diff algorithm.

use serde::{Deserialize, Serialize};
use similar::{capture_diff_slices, Algorithm, ChangeTag, DiffOp, DiffTag, TextDiff};
use std::time::Duration;

/// Cap on the line-level diff: pathological "every line differs" inputs make Myers go O(N²) (a ~2 MB
/// such file took ~6.6 s). On timeout `similar` returns a valid—if less minimal—diff instead of freezing.
const LINE_DIFF_TIMEOUT: Duration = Duration::from_millis(750);

/// Kind of an aligned row in the side-by-side view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// Both sides equal.
    Equal,
    /// Present only on the right (added).
    Insert,
    /// Present only on the left (removed).
    Delete,
    /// Both sides present but different (with word-level ranges).
    Replace,
}

/// A character range `[start, end)` within a line (char offsets, not bytes) — for word highlighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordRange {
    pub start: u32,
    pub end: u32,
}

/// One aligned row: a left line, a right line, or both.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffRow {
    /// 1-based line number on the left (`None` if no left line in this row).
    pub left_no: Option<u32>,
    /// 1-based line number on the right (`None` if no right line in this row).
    pub right_no: Option<u32>,
    pub kind: RowKind,
    pub left: Option<String>,
    pub right: Option<String>,
    /// Changed ranges within `left` at **character** granularity (for `Replace`).
    pub left_words: Vec<WordRange>,
    /// Changed ranges within `right` at **character** granularity (for `Replace`).
    pub right_words: Vec<WordRange>,
    /// Changed ranges within `left` at **word** granularity (whole changed words; for `Replace`).
    pub left_words_w: Vec<WordRange>,
    /// Changed ranges within `right` at **word** granularity (whole changed words; for `Replace`).
    pub right_words_w: Vec<WordRange>,
}

/// Per-kind row counts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    pub equal: u32,
    pub inserted: u32,
    pub deleted: u32,
    pub replaced: u32,
}

/// The full aligned diff of two texts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub rows: Vec<DiffRow>,
    pub summary: DiffSummary,
}

fn trim_eol(s: &str) -> String {
    s.trim_end_matches(['\n', '\r']).to_string()
}

/// Compute the aligned line/word diff of two texts.
pub fn diff_text(left: &str, right: &str) -> FileDiff {
    let diff = TextDiff::configure()
        .timeout(LINE_DIFF_TIMEOUT)
        .diff_lines(left, right);
    let lo = diff.old_slices();
    let ro = diff.new_slices();

    let mut rows = Vec::new();
    let mut summary = DiffSummary::default();

    for op in diff.ops() {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        match tag {
            DiffTag::Equal => {
                for (oi, ni) in old_range.zip(new_range) {
                    rows.push(DiffRow {
                        left_no: Some((oi + 1) as u32),
                        right_no: Some((ni + 1) as u32),
                        kind: RowKind::Equal,
                        left: Some(trim_eol(lo[oi])),
                        right: Some(trim_eol(ro[ni])),
                        left_words: Vec::new(),
                        right_words: Vec::new(),
                        left_words_w: Vec::new(),
                        right_words_w: Vec::new(),
                    });
                    summary.equal += 1;
                }
            }
            DiffTag::Delete => {
                for oi in old_range {
                    rows.push(delete_row(oi, trim_eol(lo[oi])));
                    summary.deleted += 1;
                }
            }
            DiffTag::Insert => {
                for ni in new_range {
                    rows.push(insert_row(ni, trim_eol(ro[ni])));
                    summary.inserted += 1;
                }
            }
            DiffTag::Replace => {
                let olds: Vec<usize> = old_range.collect();
                let news: Vec<usize> = new_range.collect();
                // Pair lines within the block by *similarity* (not position), so e.g. `]` aligns with
                // `],` (comma added) rather than with an unrelated inserted line next to it.
                for (o, n) in align_replace(&olds, &news, lo, ro) {
                    match (o, n) {
                        (Some(oi), Some(ni)) => {
                            let l = trim_eol(lo[oi]);
                            let r = trim_eol(ro[ni]);
                            let (lw, rw) = char_ranges(&l, &r);
                            let (lww, rww) = word_ranges(&l, &r);
                            rows.push(DiffRow {
                                left_no: Some((oi + 1) as u32),
                                right_no: Some((ni + 1) as u32),
                                kind: RowKind::Replace,
                                left: Some(l),
                                right: Some(r),
                                left_words: lw,
                                right_words: rw,
                                left_words_w: lww,
                                right_words_w: rww,
                            });
                            summary.replaced += 1;
                        }
                        (Some(oi), None) => {
                            rows.push(delete_row(oi, trim_eol(lo[oi])));
                            summary.deleted += 1;
                        }
                        (None, Some(ni)) => {
                            rows.push(insert_row(ni, trim_eol(ro[ni])));
                            summary.inserted += 1;
                        }
                        (None, None) => {}
                    }
                }
            }
        }
    }

    FileDiff { rows, summary }
}

fn delete_row(oi: usize, text: String) -> DiffRow {
    DiffRow {
        left_no: Some((oi + 1) as u32),
        right_no: None,
        kind: RowKind::Delete,
        left: Some(text),
        right: None,
        left_words: Vec::new(),
        right_words: Vec::new(),
        left_words_w: Vec::new(),
        right_words_w: Vec::new(),
    }
}

fn insert_row(ni: usize, text: String) -> DiffRow {
    DiffRow {
        left_no: None,
        right_no: Some((ni + 1) as u32),
        kind: RowKind::Insert,
        left: None,
        right: Some(text),
        left_words: Vec::new(),
        right_words: Vec::new(),
        left_words_w: Vec::new(),
        right_words_w: Vec::new(),
    }
}

/// Above this many DP cells we skip similarity alignment and pair positionally — keeps a fully
/// rewritten large block from going quadratic (64×64 = 4096).
const ALIGN_CELL_CAP: usize = 4096;
/// Minimum line similarity (0..=1) to pair two lines as one `Replace` row instead of delete + insert.
const PAIR_THRESHOLD: f32 = 0.5;

/// Pair the left/right lines of a Replace block. Returns `(old_idx, new_idx)` per output row: both
/// `Some` = a paired (replaced) line, left-only = delete, right-only = insert. Uses a small DP that
/// maximizes total line similarity over monotonic above-threshold pairings (Needleman–Wunsch with
/// zero-cost gaps), so similar lines line up even when shifted by inserts/deletes around them.
fn align_replace(
    olds: &[usize],
    news: &[usize],
    lo: &[&str],
    ro: &[&str],
) -> Vec<(Option<usize>, Option<usize>)> {
    let m = olds.len();
    let n = news.len();
    if m == 0 || n == 0 || m * n > ALIGN_CELL_CAP {
        return positional_pairs(olds, news);
    }

    let lt: Vec<String> = olds.iter().map(|&oi| trim_eol(lo[oi])).collect();
    let rt: Vec<String> = news.iter().map(|&ni| trim_eol(ro[ni])).collect();
    let mut sim = vec![0f32; m * n];
    for i in 0..m {
        for j in 0..n {
            sim[i * n + j] = line_similarity(&lt[i], &rt[j]);
        }
    }

    // dp[i][j] = best score aligning the first i left lines with the first j right lines.
    let stride = n + 1;
    let idx = |i: usize, j: usize| i * stride + j;
    let mut dp = vec![0f32; (m + 1) * stride];
    for i in 1..=m {
        for j in 1..=n {
            let s = sim[(i - 1) * n + (j - 1)];
            let mut best = dp[idx(i - 1, j)].max(dp[idx(i, j - 1)]); // gap: delete or insert
            if s >= PAIR_THRESHOLD {
                best = best.max(dp[idx(i - 1, j - 1)] + s); // pair these two lines
            }
            dp[idx(i, j)] = best;
        }
    }

    // Backtrack. dp cells hold exactly one of the candidate values, so `==` is reliable here.
    let mut out: Vec<(Option<usize>, Option<usize>)> = Vec::with_capacity(m + n);
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        let cur = dp[idx(i, j)];
        if i > 0 && j > 0 {
            let s = sim[(i - 1) * n + (j - 1)];
            if s >= PAIR_THRESHOLD && cur == dp[idx(i - 1, j - 1)] + s {
                out.push((Some(olds[i - 1]), Some(news[j - 1])));
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && cur == dp[idx(i - 1, j)] {
            out.push((Some(olds[i - 1]), None)); // delete
            i -= 1;
        } else {
            out.push((None, Some(news[j - 1]))); // insert
            j -= 1;
        }
    }
    out.reverse();
    out
}

/// Naïve positional pairing (the fallback for empty or oversized blocks).
fn positional_pairs(olds: &[usize], news: &[usize]) -> Vec<(Option<usize>, Option<usize>)> {
    let n = olds.len().max(news.len());
    (0..n)
        .map(|k| (olds.get(k).copied(), news.get(k).copied()))
        .collect()
}

/// Similarity of two lines in `0.0..=1.0` (char-level `2·matches / total`, via `similar`).
fn line_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    TextDiff::from_chars(a, b).ratio()
}

/// Above this line length we skip intra-line highlighting (char diff is O(n²); also avoids noise on
/// minified/one-line files). The row still shows as a whole-line change.
const INLINE_DIFF_CAP: usize = 4000;

/// Append a range, coalescing with the previous one if they touch (char diff yields one change per
/// char; we merge runs into contiguous highlighted spans).
fn push_range(v: &mut Vec<WordRange>, start: u32, end: u32) {
    if let Some(last) = v.last_mut() {
        if last.end == start {
            last.end = end;
            return;
        }
    }
    v.push(WordRange { start, end });
}

/// Intra-line changed ranges (char offsets) at **character** granularity within two replaced lines.
/// (NB: `similar::from_words` splits only on whitespace, so it would mark a whole space-free CSV/JSONL
/// line as changed — hence a char diff.) Runs of changed chars are coalesced into contiguous spans.
fn char_ranges(a: &str, b: &str) -> (Vec<WordRange>, Vec<WordRange>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    if a.chars().count().max(b.chars().count()) > INLINE_DIFF_CAP {
        return (left, right); // too long → skip intra-line highlight (shown as a whole-line change)
    }
    let wd = TextDiff::from_chars(a, b);
    let mut ai: u32 = 0;
    let mut bi: u32 = 0;
    for change in wd.iter_all_changes() {
        let len = change.value().chars().count() as u32;
        match change.tag() {
            ChangeTag::Equal => {
                ai += len;
                bi += len;
            }
            ChangeTag::Delete => {
                push_range(&mut left, ai, ai + len);
                ai += len;
            }
            ChangeTag::Insert => {
                push_range(&mut right, bi, bi + len);
                bi += len;
            }
        }
    }
    (left, right)
}

/// Split a line into maximal runs of word chars (`alphanumeric` or `_`) vs runs of separators — so a
/// change inside a token highlights the whole token (cleaner than per-char on long strings/numbers).
fn tokenize_words(s: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let w = is_word(chars[i].1);
        let start = chars[i].0;
        let mut j = i + 1;
        while j < chars.len() && is_word(chars[j].1) == w {
            j += 1;
        }
        let end = if j < chars.len() { chars[j].0 } else { s.len() };
        toks.push(&s[start..end]);
        i = j;
    }
    toks
}

/// Intra-line changed ranges at **word** granularity: tokenize each line, diff the token sequences, and
/// map changed tokens back to char ranges (coalesced). A change inside a word marks the whole word.
fn word_ranges(a: &str, b: &str) -> (Vec<WordRange>, Vec<WordRange>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    if a.chars().count().max(b.chars().count()) > INLINE_DIFF_CAP {
        return (left, right);
    }
    let at = tokenize_words(a);
    let bt = tokenize_words(b);
    let alen: Vec<u32> = at.iter().map(|t| t.chars().count() as u32).collect();
    let blen: Vec<u32> = bt.iter().map(|t| t.chars().count() as u32).collect();
    let span = |lens: &[u32], idx: usize, n: usize| -> u32 { lens[idx..idx + n].iter().sum() };
    let mut ai: u32 = 0;
    let mut bi: u32 = 0;
    for op in capture_diff_slices(Algorithm::Myers, &at, &bt) {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                ai += span(&alen, old_index, len);
                bi += span(&blen, new_index, len);
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                let d = span(&alen, old_index, old_len);
                push_range(&mut left, ai, ai + d);
                ai += d;
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                let d = span(&blen, new_index, new_len);
                push_range(&mut right, bi, bi + d);
                bi += d;
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let dl = span(&alen, old_index, old_len);
                let dr = span(&blen, new_index, new_len);
                push_range(&mut left, ai, ai + dl);
                push_range(&mut right, bi, bi + dr);
                ai += dl;
                bi += dr;
            }
        }
    }
    (left, right)
}

/// A contiguous region of changes with surrounding context lines — one "hunk" in unified-diff terms.
/// Groups several non-equal rows (and the context lines bridging them when two change clusters are
/// close together) into a single displayable block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    /// The aligned rows for this hunk: context + non-equal + context.
    pub rows: Vec<DiffRow>,
    /// 1-based first left-side line number covered by this hunk (including context).
    pub left_start: u32,
    /// 1-based first right-side line number covered by this hunk (including context).
    pub right_start: u32,
}

/// Result of a hunks-only diff (large-file mode).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDiffHunks {
    pub hunks: Vec<DiffHunk>,
    /// Summary counts across all returned hunks (not the full file, unless `is_complete`).
    pub summary: DiffSummary,
    /// Whether all changes in both texts were captured (false if `max_hunks` was hit before EOF).
    pub is_complete: bool,
    /// How many hunks exist in total (only meaningful when `is_complete = true`; 0 otherwise).
    pub total_hunks: usize,
    /// Index to pass as `start_hunk` in the next call for pagination (`None` when complete).
    pub next_hunk_index: Option<usize>,
}

/// True if `data` looks like binary content (contains a null byte in the first 8 000 bytes).
pub fn is_binary_bytes(data: &[u8]) -> bool {
    data[..data.len().min(8000)].contains(&0u8)
}

/// Compute only the changed regions (hunks) of a diff, each padded with `context` equal lines.
/// Adjacent hunk clusters separated by ≤ `2×context` equal lines are merged into one hunk
/// (standard unified-diff behaviour: the context of one overlaps the context of the other).
/// Returns at most `max_hunks` hunks starting from hunk index `start_hunk` (0 for the first page).
/// `is_complete` is false if more hunks remain; `next_hunk_index` gives the cursor for the next call.
pub fn diff_hunks(left: &str, right: &str, context: usize, max_hunks: usize, start_hunk: usize) -> FileDiffHunks {
    // Compute the full aligned diff first (required by Myers — we can't skip this).
    // We then extract only the hunk windows; the equal rows in between are discarded.
    let full = diff_text(left, right);
    let rows = &full.rows;
    if rows.is_empty() {
        return FileDiffHunks { hunks: Vec::new(), summary: DiffSummary::default(), is_complete: true, total_hunks: 0, next_hunk_index: None };
    }

    // Find the index spans [start, end) of non-equal runs in the row array.
    let mut change_spans: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if rows[i].kind != RowKind::Equal {
            let start = i;
            while i < rows.len() && rows[i].kind != RowKind::Equal { i += 1; }
            change_spans.push((start, i));
        } else {
            i += 1;
        }
    }

    if change_spans.is_empty() {
        return FileDiffHunks { hunks: Vec::new(), summary: DiffSummary::default(), is_complete: true, total_hunks: 0, next_hunk_index: None };
    }

    // Expand each span by `context` lines on each side, then merge overlapping/adjacent windows.
    let n = rows.len();
    let mut windows: Vec<(usize, usize)> = change_spans.iter()
        .map(|&(s, e)| (s.saturating_sub(context), (e + context).min(n)))
        .collect();
    // Merge windows whose ranges overlap or touch.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (ws, we) in windows.drain(..) {
        if let Some(last) = merged.last_mut() {
            if ws <= last.1 {
                last.1 = last.1.max(we);
                continue;
            }
        }
        merged.push((ws, we));
    }

    let total_hunks = merged.len();
    let skip = start_hunk.min(total_hunks);
    let remaining = total_hunks - skip;
    let is_complete = remaining <= max_hunks;
    let take = remaining.min(max_hunks);
    let next_hunk_index = if is_complete { None } else { Some(skip + take) };

    let mut hunks = Vec::with_capacity(take);
    let mut summary = DiffSummary::default();
    for (ws, we) in merged.into_iter().skip(skip).take(take) {
        let hunk_rows: Vec<DiffRow> = rows[ws..we].to_vec();
        for r in &hunk_rows {
            match r.kind {
                RowKind::Equal   => summary.equal += 1,
                RowKind::Insert  => summary.inserted += 1,
                RowKind::Delete  => summary.deleted += 1,
                RowKind::Replace => summary.replaced += 1,
            }
        }
        let left_start = hunk_rows.iter().find_map(|r| r.left_no).unwrap_or(1);
        let right_start = hunk_rows.iter().find_map(|r| r.right_no).unwrap_or(1);
        hunks.push(DiffHunk { rows: hunk_rows, left_start, right_start });
    }

    FileDiffHunks { hunks, summary, is_complete, total_hunks: if is_complete { total_hunks } else { 0 }, next_hunk_index }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_all_equal() {
        let d = diff_text("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(
            d.summary,
            DiffSummary {
                equal: 3,
                ..Default::default()
            }
        );
        assert!(d.rows.iter().all(|r| r.kind == RowKind::Equal));
    }

    #[test]
    fn replaced_line_has_word_ranges() {
        let d = diff_text("hello world\n", "hello brave world\n");
        let replace: Vec<_> = d
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Replace)
            .collect();
        assert_eq!(replace.len(), 1);
        // the right side gained a word → at least one right_words range, none deleted on the left
        assert!(!replace[0].right_words.is_empty());
        assert!(replace[0].left_words.is_empty());
    }

    #[test]
    fn replace_block_aligns_by_similarity() {
        // Closing bracket gains a comma and a line is inserted before it (the config.json case).
        // Positional pairing would align `]` with the unrelated inserted line; similarity alignment
        // must pair `]` ↔ `],` (a small change) and surface `  "b"` as an Insert.
        let left = "[\n  \"a\"\n]\n";
        let right = "[\n  \"a\",\n  \"b\"\n],\n";
        let d = diff_text(left, right);

        let bracket = d
            .rows
            .iter()
            .find(|r| r.left.as_deref() == Some("]"))
            .expect("a row whose left line is `]`");
        assert_eq!(bracket.kind, RowKind::Replace);
        assert_eq!(bracket.right.as_deref(), Some("],"));

        assert!(d
            .rows
            .iter()
            .any(|r| r.kind == RowKind::Insert && r.right.as_deref() == Some("  \"b\"")));
    }

    #[test]
    fn word_change_in_a_spaceless_line_is_localized() {
        // A repetitive, space-free CSV line with one changed cell. `from_words` would mark the whole
        // line (no spaces → one token); the char-level diff must localize it to just the changed run.
        let mut l = String::from("row3");
        let mut r = String::from("row3");
        for c in 1..=25 {
            l.push_str(&format!(",val_3_{c}"));
            if c == 20 {
                r.push_str(",val_3_20_CHANGED");
            } else {
                r.push_str(&format!(",val_3_{c}"));
            }
        }
        let d = diff_text(&format!("{l}\n"), &format!("{r}\n"));
        let row = d.rows.iter().find(|x| x.kind == RowKind::Replace).unwrap();
        let changed: u32 = row.right_words.iter().map(|w| w.end - w.start).sum();
        let total = row.right.as_ref().unwrap().chars().count() as u32;
        assert!(row.left_words.is_empty(), "nothing was deleted on the left");
        assert!(
            changed > 0 && changed < total / 4,
            "change localized: {changed} of {total}"
        );
    }

    #[test]
    fn word_mode_highlights_whole_token() {
        // "foo123bar" → "foo456bar": char-level marks only the 3 differing digits; word-level marks the
        // whole alphanumeric token on both sides.
        let d = diff_text("foo123bar\n", "foo456bar\n");
        let row = d.rows.iter().find(|r| r.kind == RowKind::Replace).unwrap();
        let ch: u32 = row.right_words.iter().map(|w| w.end - w.start).sum();
        assert_eq!(ch, 3, "char-level localizes to the 3 changed digits");
        assert_eq!(row.left_words_w, vec![WordRange { start: 0, end: 9 }]);
        assert_eq!(row.right_words_w, vec![WordRange { start: 0, end: 9 }]);
    }

    #[test]
    fn insert_and_delete_lines() {
        let d = diff_text("a\nb\n", "a\nb\nc\n");
        assert_eq!(d.summary.inserted, 1);
        assert_eq!(d.summary.deleted, 0);
        let d2 = diff_text("a\nb\nc\n", "a\nc\n");
        assert_eq!(d2.summary.deleted, 1);
    }

    // ---- diff_hunks tests ----

    fn lines(n: u32) -> String {
        (1..=n).map(|i| format!("line{}\n", i)).collect()
    }

    #[test]
    fn identical_files_produce_no_hunks() {
        let t = lines(50);
        let r = diff_hunks(&t, &t, 3, 100, 0);
        assert!(r.hunks.is_empty());
        assert!(r.is_complete);
    }

    #[test]
    fn single_change_produces_at_least_one_hunk_with_context() {
        // 50-line file; only line 25 differs. `align_replace` may split the change into a Delete +
        // Insert pair (2 adjacent rows), but with context=3 the windows always merge into 1 hunk.
        let left: Vec<String> = (1..=50).map(|i| format!("line{}\n", i)).collect();
        let mut right = left.clone();
        right[24] = "CHANGED\n".to_string();
        let l_str: String = left.concat();
        let r_str: String = right.concat();

        let result = diff_hunks(&l_str, &r_str, 3, 100, 0);
        assert!(result.is_complete);
        // With context=3, the change at line 25 and any adjacent split rows merge into a single
        // window — we get at most 2 hunks (one before and one for the change) but always at least 1.
        assert!(!result.hunks.is_empty());
        // Every changed line should be represented across the hunks.
        let changed_count: usize = result.hunks.iter()
            .flat_map(|h| h.rows.iter())
            .filter(|r| r.kind != RowKind::Equal)
            .count();
        assert!(changed_count >= 1);
        // The total row count across all hunks must be more than 1 (context lines present).
        let total_rows: usize = result.hunks.iter().map(|h| h.rows.len()).sum();
        assert!(total_rows > 1);
    }

    #[test]
    fn two_close_changes_are_merged_into_one_hunk() {
        // Changes at lines 10 and 14; context=3 → gap between them = 3 lines < 2×3 → merged.
        let left: Vec<String> = (1..=30).map(|i| format!("line{}\n", i)).collect();
        let mut right = left.clone();
        right[9] = "CHANGE_A\n".to_string();
        right[13] = "CHANGE_B\n".to_string();
        let r = diff_hunks(&left.concat(), &right.concat(), 3, 100, 0);
        assert_eq!(r.hunks.len(), 1, "close changes should merge into one hunk");
    }

    #[test]
    fn two_far_changes_produce_two_hunks() {
        // Changes at lines 5 and 25; gap > 2×context → separate hunks.
        let left: Vec<String> = (1..=40).map(|i| format!("line{}\n", i)).collect();
        let mut right = left.clone();
        right[4] = "CHANGE_A\n".to_string();
        right[24] = "CHANGE_B\n".to_string();
        let r = diff_hunks(&left.concat(), &right.concat(), 3, 100, 0);
        assert_eq!(r.hunks.len(), 2);
        assert!(r.is_complete);
    }

    #[test]
    fn max_hunks_limit_sets_is_complete_false() {
        // 10 changes spread out → 10 hunks, but max_hunks=3.
        let left: Vec<String> = (1..=100).map(|i| format!("line{}\n", i)).collect();
        let mut right = left.clone();
        for i in (0..100).step_by(10) { right[i] = format!("CHANGED{}\n", i); }
        let r = diff_hunks(&left.concat(), &right.concat(), 3, 3, 0);
        assert_eq!(r.hunks.len(), 3);
        assert!(!r.is_complete);
        assert_eq!(r.total_hunks, 0); // unknown when not complete
        assert_eq!(r.next_hunk_index, Some(3)); // cursor for "Load more"
    }

    #[test]
    fn pagination_via_start_hunk_walks_all_hunks_without_overlap() {
        // 10 well-separated changes → 10 hunks. Page through them 3 at a time and confirm we see
        // each hunk exactly once, in order, with the cursor advancing correctly.
        let left: Vec<String> = (1..=100).map(|i| format!("line{}\n", i)).collect();
        let mut right = left.clone();
        for i in (0..100).step_by(10) { right[i] = format!("CHANGED{}\n", i); }
        let (l, r) = (left.concat(), right.concat());

        let mut seen_starts = Vec::new();
        let mut start = 0;
        loop {
            let page = diff_hunks(&l, &r, 3, 3, start);
            for h in &page.hunks { seen_starts.push(h.left_start); }
            match page.next_hunk_index {
                Some(n) => start = n,
                None => break,
            }
        }
        // 10 hunks total, strictly increasing start lines, no duplicates.
        assert_eq!(seen_starts.len(), 10);
        assert!(seen_starts.windows(2).all(|w| w[0] < w[1]), "hunks out of order or duplicated");
    }

    #[test]
    fn hunk_left_start_is_correct() {
        // Single change at line 10 with context=3 → hunk starts at line 7.
        let left: Vec<String> = (1..=20).map(|i| format!("line{}\n", i)).collect();
        let mut right = left.clone();
        right[9] = "CHANGED\n".to_string();
        let r = diff_hunks(&left.concat(), &right.concat(), 3, 100, 0);
        assert_eq!(r.hunks.len(), 1);
        assert_eq!(r.hunks[0].left_start, 7); // 10 - 3 context
    }
}
