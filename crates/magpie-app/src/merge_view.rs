/// Map a separator selector index to the actual separator string.
/// 0 = newline, 1 = space, 2 = comma-space; anything else = newline.
pub fn separator_str(idx: i32) -> &'static str {
    match idx {
        1 => " ",
        2 => ", ",
        _ => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_indices() {
        assert_eq!(separator_str(0), "\n");
        assert_eq!(separator_str(1), " ");
        assert_eq!(separator_str(2), ", ");
        assert_eq!(separator_str(7), "\n");
    }
}
