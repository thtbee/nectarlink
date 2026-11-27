// SPDX-License-Identifier: GPL-3.0-or-later
//! What the menu shows: the app keeps it in the registry, under [`KEY`] in
//! the user's hive, whenever its paired phones change; the DLL reads it each
//! time Explorer opens a menu. Shared by source with the app.

/// The key, under `HKEY_CURRENT_USER`.
pub const KEY: &str = r"Software\Nectarlink\SendMenu";
/// `REG_SZ`: the entry's title when it opens a list of phones.
pub const TITLE: &str = "Title";
/// `REG_MULTI_SZ`: one string per phone, from [`encode`].
pub const PHONES: &str = "Phones";

/// A phone the menu sends to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    /// Its device ID, which `--send-to` takes.
    pub id: String,
    /// Its name, its entry in the list of phones.
    pub name: String,
    /// The entry's title when it's the only phone ("Send to Pixel with NectarLink").
    pub title: String,
}

/// The strings for [`PHONES`]: ID, name and title, split by tabs.
pub fn encode(phones: &[Phone]) -> Vec<String> {
    phones
        .iter()
        .filter(|p| valid_id(&p.id))
        .map(|p| format!("{}\t{}\t{}", p.id, clean(&p.name), clean(&p.title)))
        .collect()
}

/// The phones in [`PHONES`]; anything that doesn't read as one is left out.
pub fn decode<S: AsRef<str>>(lines: &[S]) -> Vec<Phone> {
    lines
        .iter()
        .filter_map(|line| {
            let mut parts = line.as_ref().split('\t');
            let (id, name, title) = (parts.next()?, parts.next()?, parts.next()?);
            (valid_id(id) && !name.trim().is_empty() && parts.next().is_none()).then(|| Phone {
                id: id.to_owned(),
                name: name.to_owned(),
                title: if title.trim().is_empty() { name.to_owned() } else { title.to_owned() },
            })
        })
        .collect()
}

/// A device ID as the app writes them: letters and digits only, so it can
/// never read as an option or a file.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// One line of text, without the tabs that split the fields.
fn clean(text: &str) -> String {
    let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(id: &str, name: &str) -> Phone {
        Phone { id: id.into(), name: name.into(), title: format!("Send to {name} with NectarLink") }
    }

    #[test]
    fn phones_come_back_as_written() {
        let phones = vec![phone("0a1b2c", "Pixel 8"), phone("ff00", "Galaxy Tab")];
        assert_eq!(decode(&encode(&phones)), phones);
    }

    #[test]
    fn a_tab_or_line_break_in_a_name_is_a_space() {
        let written = encode(&[phone("0a", "My\tphone\n")]);
        assert_eq!(decode(&written)[0].name, "My phone");
    }

    #[test]
    fn what_isnt_a_phone_is_left_out() {
        assert!(encode(&[phone("--help", "Pixel")]).is_empty());
        assert!(
            decode(&["", "0a", "0a\tPixel", "0a\t \tSend", "x y\tPixel\tSend", "0a\tPixel\tSend\textra"])
                .is_empty()
        );
        // No title: the name stands in.
        assert_eq!(decode(&["0a\tPixel\t"])[0].title, "Pixel");
    }
}
