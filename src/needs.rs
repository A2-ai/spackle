use std::collections::HashMap;

pub trait Needy {
    fn key(&self) -> String;

    fn is_enabled(&self, data: &HashMap<String, String>) -> bool;

    /// Returns true if all entries in *needs* are satisfied given the provided user inputs
    /// Needy items are satisfied if they are enabled (either by the user or by default) and their needs are satisfied
    /// Needy items are not checked for recursion, so be careful with circular dependencies
    fn is_satisfied(&self, items: &Vec<&dyn Needy>, data: &HashMap<String, String>) -> bool;
}

pub fn is_satisfied(
    needs: &Vec<String>,
    items: &Vec<&dyn Needy>,
    data: &HashMap<String, String>,
) -> bool {
    needs
        .iter()
        .all(|key| match items.iter().find(|h| h.key() == *key) {
            Some(item) => item.is_enabled(data) && item.is_satisfied(items, data),
            None => false,
        })
}
