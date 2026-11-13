//! The identifier a column name takes in a generated enum.

use proc_macro2::Ident;
use quote::format_ident;

/// The identifier a wire spelling takes, e.g. `issueDate` -> `IssueDate`, `ilike` -> `Ilike`.
pub(crate) fn pascal_name(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let characters: Vec<char> = name.chars().collect();
    for (index, character) in characters.iter().enumerate() {
        if !character.is_ascii_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        if character.is_ascii_uppercase() && index > 0 {
            let previous = characters[index - 1];
            if (previous.is_ascii_lowercase() || previous.is_ascii_digit()) && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        }
        word.push(*character);
    }
    if !word.is_empty() {
        words.push(word);
    }
    if words.is_empty() {
        return "Unknown".to_owned();
    }
    words
        .iter()
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => {
                    first.to_ascii_uppercase().to_string()
                        + &characters.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

/// The variant a column takes in a generated enum, suffixed when it would name `Self`.
pub(crate) fn variant(name: &str) -> Ident {
    let variant = pascal_name(name);
    let suffix = if variant == "Self" { "_" } else { "" };
    format_ident!("{variant}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::pascal_name;

    #[test]
    fn names_a_column_as_its_field_type_does() {
        assert_eq!(pascal_name("issueDate"), "IssueDate");
        assert_eq!(pascal_name("ilike"), "Ilike");
        assert_eq!(pascal_name("notIlike"), "NotIlike");
        assert_eq!(
            pascal_name("tenant_shared_permission"),
            "TenantSharedPermission"
        );
    }
}
