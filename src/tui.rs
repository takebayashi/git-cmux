use anyhow::Result;
use dialoguer::{Input, Select};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerItemKind {
    Action,
    Item,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub primary: String,
    pub secondary: Option<String>,
    pub kind: PickerItemKind,
}

pub fn pick_row(prompt: &str, rows: &[PickerRow], default: usize) -> Result<Option<usize>> {
    let items = render_rows(rows);

    Select::new()
        .with_prompt(prompt)
        .items(&items)
        .default(default)
        .interact_opt()
        .map_err(Into::into)
}

pub fn prompt_text(prompt: &str) -> Result<Option<String>> {
    let value: String = Input::new()
        .with_prompt(prompt)
        .allow_empty(true)
        .interact_text()?;
    let value = value.trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn render_rows(rows: &[PickerRow]) -> Vec<String> {
    rows.iter().map(render_row).collect()
}

fn render_row(row: &PickerRow) -> String {
    match row.kind {
        PickerItemKind::Action => format!("+ {}", row.primary),
        PickerItemKind::Item => match row.secondary.as_deref() {
            Some(secondary) => format!("{:<36}  {}", row.primary, secondary),
            None => row.primary.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{PickerItemKind, PickerRow, render_row};

    #[test]
    fn render_row_formats_item_with_secondary_text() {
        let row = PickerRow {
            primary: "main".to_string(),
            secondary: Some("/repo/.worktrees/main".to_string()),
            kind: PickerItemKind::Item,
        };

        let rendered = render_row(&row);

        assert!(rendered.contains("main"));
        assert!(rendered.contains("/repo/.worktrees/main"));
    }

    #[test]
    fn render_row_formats_item_without_secondary_text() {
        let row = PickerRow {
            primary: "detached".to_string(),
            secondary: None,
            kind: PickerItemKind::Item,
        };

        assert_eq!(render_row(&row), "detached");
    }
}
