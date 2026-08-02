use magpie_app::mask_view::{mask_render, should_mask, MaskRules};

#[test]
fn end_to_end_masking_shape() {
    let rules = MaskRules::build(&["1Password".into()], &[r"^ghp_".into()], false);
    assert!(should_mask(&rules, Some("1Password"), "hunter2"));
    assert!(should_mask(&rules, None, "ghp_tokenvalue"));
    assert!(!should_mask(&rules, Some("Notes"), "grocery list"));
    assert_eq!(
        mask_render("ghp_secret", 3),
        format!("ghp{}", "•".repeat(7))
    );
}
