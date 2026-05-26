use gpui::TextRun;

const NUMBERED_PREFIXES_1: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const NUMBERED_PREFIXES_2: &str = "abcdefghijklmnopqrstuvwxyz";

const BULLETS: [&str; 5] = ["•", "◦", "▪", "‣", "⁃"];

/// Returns the prefix for a list item.
pub(super) fn list_item_prefix(ix: usize, ordered: bool, depth: usize) -> String {
    if ordered {
        if depth == 0 {
            return format!("{}. ", ix + 1);
        }

        if depth == 1 {
            return format!(
                "{}. ",
                NUMBERED_PREFIXES_1
                    .chars()
                    .nth(ix % NUMBERED_PREFIXES_1.len())
                    .unwrap()
            );
        } else {
            return format!(
                "{}. ",
                NUMBERED_PREFIXES_2
                    .chars()
                    .nth(ix % NUMBERED_PREFIXES_2.len())
                    .unwrap()
            );
        }
    } else {
        let depth = depth.min(BULLETS.len() - 1);
        let bullet = BULLETS[depth];
        return format!("{} ", bullet);
    }
}

pub(super) fn normalize_runs_for_text(text: &str, runs: Vec<TextRun>) -> Vec<TextRun> {
    if text.is_empty() {
        return vec![];
    }

    let mut normalized = Vec::with_capacity(runs.len());
    let mut offset = 0;

    for run in runs {
        if offset >= text.len() {
            break;
        }

        let mut end = offset.saturating_add(run.len).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }

        if end <= offset {
            continue;
        }

        normalized.push(TextRun {
            len: end - offset,
            ..run
        });
        offset = end;
    }

    if offset < text.len() {
        if let Some(last_run) = normalized.last_mut() {
            last_run.len += text.len() - offset;
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use gpui::TextRun;

    use super::{list_item_prefix, normalize_runs_for_text};

    #[test]
    fn test_list_item_prefix() {
        assert_eq!(list_item_prefix(0, true, 0), "1. ");
        assert_eq!(list_item_prefix(1, true, 0), "2. ");
        assert_eq!(list_item_prefix(2, true, 0), "3. ");
        assert_eq!(list_item_prefix(10, true, 0), "11. ");
        assert_eq!(list_item_prefix(0, true, 1), "A. ");
        assert_eq!(list_item_prefix(1, true, 1), "B. ");
        assert_eq!(list_item_prefix(2, true, 1), "C. ");
        assert_eq!(list_item_prefix(0, true, 2), "a. ");
        assert_eq!(list_item_prefix(1, true, 2), "b. ");
        assert_eq!(list_item_prefix(6, true, 2), "g. ");
        assert_eq!(list_item_prefix(0, true, 1), "A. ");
        assert_eq!(list_item_prefix(0, true, 2), "a. ");
        assert_eq!(list_item_prefix(0, false, 0), "• ");
        assert_eq!(list_item_prefix(0, false, 1), "◦ ");
        assert_eq!(list_item_prefix(0, false, 2), "▪ ");
        assert_eq!(list_item_prefix(0, false, 3), "‣ ");
        assert_eq!(list_item_prefix(0, false, 4), "⁃ ");
    }

    #[test]
    fn test_normalize_runs_for_text_keeps_utf8_boundaries() {
        let run = TextRun {
            len: 0,
            font: gpui::font(".SystemUIFont"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text = "- normal 渲染 -> supported";

        let runs = normalize_runs_for_text(
            text,
            vec![
                TextRun {
                    len: 13,
                    ..run.clone()
                },
                TextRun { len: 20, ..run },
            ],
        );

        let mut offset = 0;
        for run in &runs {
            offset += run.len;
            assert!(
                text.is_char_boundary(offset),
                "run boundary {offset} should be a UTF-8 char boundary"
            );
        }

        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
    }
}
