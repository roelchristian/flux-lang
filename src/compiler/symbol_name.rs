// Linked names encode a canonical path, but their source names are display metadata.
fn linked_prefix(name: &str) -> Option<usize> {
    let head = name.strip_prefix('\0')?;
    let (tag, _) = head.split_once(':')?;
    (!tag.is_empty()
        && tag.len() % 2 == 0
        && tag
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(tag.len() + 2)
}

pub(crate) fn visible_symbol(name: &str) -> &str {
    linked_prefix(name).map_or(name, |prefix| &name[prefix..])
}
