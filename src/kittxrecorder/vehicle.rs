//! Offline suggestions for caller-provided vehicle metadata, with custom entries.
use gpui_kit::component::{
    IndexPath,
    searchable_list::{SearchableListChange, SearchableListDelegate, SearchableListItem},
};
use gpui_kit::{App, Task, Window};

// Suggestions only: this is neither a complete vehicle catalog nor a VIN decoder.
const VEHICLES: &[(&str, &[&str])] = &[
    ("Alfa Romeo", &["Giulia", "Giulietta", "Stelvio"]),
    ("Audi", &["A1", "A3", "A4", "A6", "Q3", "Q5"]),
    (
        "BMW",
        &["1 Series", "3 Series", "5 Series", "X1", "X3", "X5"],
    ),
    ("Chevrolet", &["Aveo", "Cruze", "Malibu", "Spark"]),
    ("Citroën", &["C3", "C4", "C5"]),
    ("Dacia", &["Duster", "Logan", "Sandero"]),
    ("Fiat", &["500", "Panda", "Punto", "Tipo"]),
    ("Ford", &["Fiesta", "Focus", "Kuga", "Mondeo"]),
    ("Honda", &["Accord", "Civic", "CR-V", "Jazz"]),
    ("Hyundai", &["i10", "i20", "i30", "Santa Fe", "Tucson"]),
    ("Jeep", &["Cherokee", "Compass", "Renegade", "Wrangler"]),
    ("Kia", &["Ceed", "Picanto", "Sorento", "Sportage"]),
    ("Land Rover", &["Defender", "Discovery", "Range Rover"]),
    ("Lexus", &["CT", "IS", "NX", "RX"]),
    ("MINI", &["Clubman", "Cooper", "Countryman"]),
    ("Mazda", &["2", "3", "6", "CX-5"]),
    (
        "Mercedes-Benz",
        &["A-Class", "C-Class", "E-Class", "S-Class", "GLC"],
    ),
    ("Mitsubishi", &["Colt", "Lancer", "Outlander"]),
    ("Nissan", &["Juke", "Leaf", "Micra", "Qashqai"]),
    ("Opel", &["Astra", "Corsa", "Insignia", "Mokka"]),
    ("Peugeot", &["208", "308", "508", "2008", "3008"]),
    ("Porsche", &["911", "Cayenne", "Macan"]),
    ("Renault", &["Captur", "Clio", "Megane", "Scenic"]),
    ("SEAT", &["Ateca", "Ibiza", "Leon"]),
    ("Subaru", &["Forester", "Impreza", "Outback"]),
    ("Suzuki", &["Swift", "SX4", "Vitara"]),
    ("Tesla", &["Model 3", "Model S", "Model X", "Model Y"]),
    (
        "Toyota",
        &["Avensis", "Camry", "Corolla", "Prius", "RAV4", "Yaris"],
    ),
    (
        "Volkswagen",
        &["Golf", "Passat", "Polo", "Tiguan", "Touareg", "Touran"],
    ),
    ("Volvo", &["S60", "S90", "V60", "V90", "XC60", "XC90"]),
    ("Škoda", &["Fabia", "Karoq", "Kodiaq", "Octavia", "Superb"]),
];

pub(crate) struct VehicleOptions {
    suggestions: Vec<String>,
    matches: Vec<String>,
}
impl VehicleOptions {
    fn new(items: impl IntoIterator<Item = &'static str>) -> Self {
        let suggestions: Vec<_> = items.into_iter().map(str::to_owned).collect();
        Self {
            matches: suggestions.clone(),
            suggestions,
        }
    }
    pub(crate) fn brands() -> Self {
        Self::new(VEHICLES.iter().map(|(brand, _)| *brand))
    }
    pub(crate) fn models(brand: Option<&str>) -> Self {
        Self::new(
            VEHICLES
                .iter()
                .find(|(name, _)| Some(*name) == brand)
                .into_iter()
                .flat_map(|(_, models)| models.iter().copied()),
        )
    }
}
impl SearchableListDelegate for VehicleOptions {
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
        if !query.is_empty()
            && !self
                .suggestions
                .iter()
                .any(|item| item.to_lowercase() == lowercase)
        {
            // The typed value is a selectable row; unconfirmed search text is not metadata.
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
            // Retain a custom value before Select clears the search and resolves its
            // committed index in the full list. Searches alone never add metadata.
            if !self.suggestions.contains(&value) {
                self.suggestions.push(value.clone());
            }
            selection.push((index, value));
        }
    }
}
