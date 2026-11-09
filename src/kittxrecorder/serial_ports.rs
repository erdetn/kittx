//! Editable serial connection suggestions; discovery never opens a device.
use gpui_kit::component::{
    IndexPath,
    searchable_list::{SearchableListChange, SearchableListDelegate, SearchableListItem},
};
use gpui_kit::{App, Task, Window};
use std::io;
#[cfg(any(test, all(feature = "serial", not(target_os = "android"))))]
use std::{collections::BTreeSet, path::PathBuf};

pub(crate) fn discovered_ports() -> io::Result<Vec<String>> {
    #[cfg(all(feature = "serial", not(target_os = "android")))]
    {
        serial2::SerialPort::available_ports()
            .map(|paths| port_names(paths, cfg!(target_os = "macos")))
    }
    #[cfg(not(all(feature = "serial", not(target_os = "android"))))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Serial support is disabled",
        ))
    }
}

#[cfg(any(test, all(feature = "serial", not(target_os = "android"))))]
fn port_names(paths: Vec<PathBuf>, prefer_callout: bool) -> Vec<String> {
    let names: BTreeSet<_> = paths
        .into_iter()
        .filter_map(|path| path.into_os_string().into_string().ok())
        .filter(|name| !name.is_empty())
        .collect();
    names
        .iter()
        .filter(|name| {
            !prefer_callout
                || !name
                    .strip_prefix("/dev/tty.")
                    .is_some_and(|suffix| names.contains(&format!("/dev/cu.{suffix}")))
        })
        .cloned()
        .collect()
}

pub(crate) struct SerialOptions {
    suggestions: Vec<String>,
    matches: Vec<String>,
}

impl SerialOptions {
    pub(crate) fn new(mut suggestions: Vec<String>) -> Self {
        suggestions.sort();
        suggestions.dedup();
        Self {
            matches: suggestions.clone(),
            suggestions,
        }
    }

    pub(crate) fn baud_rates() -> Self {
        // Suggestions only: support for a rate depends on the adapter and driver.
        let suggestions: Vec<_> = [
            110, 300, 600, 1200, 2400, 4800, 7200, 9600, 14400, 19200, 28800, 38400, 56000, 57600,
            76800, 115200, 128000, 153600, 230400, 250000, 256000, 460800, 500000, 576000, 921600,
            1_000_000, 1_500_000, 2_000_000, 3_000_000, 4_000_000,
        ]
        .into_iter()
        .map(|rate| rate.to_string())
        .collect();
        Self {
            matches: suggestions.clone(),
            suggestions,
        }
    }

    pub(crate) fn retain_value(&mut self, value: &str) {
        if !self.suggestions.iter().any(|item| item == value) {
            self.suggestions.push(value.to_owned());
            self.matches = self.suggestions.clone();
        }
    }
}

impl SearchableListDelegate for SerialOptions {
    type Item = String;

    fn items_count(&self, section: usize) -> usize {
        if section == 0 { self.matches.len() } else { 0 }
    }
    fn item(&self, index: IndexPath) -> Option<&String> {
        (index.section == 0)
            .then(|| self.matches.get(index.row))
            .flatten()
    }
    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.matches
            .iter()
            .position(|item| item.value() == value)
            .map(IndexPath::new)
    }
    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        let query = query.trim();
        let lowercase = query.to_lowercase();
        self.matches = self
            .suggestions
            .iter()
            .filter(|item| item.to_lowercase().contains(&lowercase))
            .cloned()
            .collect();
        if !query.is_empty() && !self.suggestions.iter().any(|item| item == query) {
            self.matches.push(query.to_owned());
        }
        Task::ready(())
    }
    fn on_will_change(
        &mut self,
        selection: &mut Vec<(IndexPath, String)>,
        changes: &[SearchableListChange],
    ) {
        let selected = changes.iter().rev().find_map(|change| match change {
            SearchableListChange::Select { index } => {
                self.item(*index).cloned().map(|value| (*index, value))
            }
            SearchableListChange::Deselect { .. } => None,
        });
        selection.clear();
        if let Some((index, value)) = selected {
            self.retain_value(&value);
            selection.push((index, value));
        }
    }
}

#[cfg(test)]
#[path = "serial_ports_tests.rs"]
mod tests;
