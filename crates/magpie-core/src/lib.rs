pub mod detect;
pub mod metrics;
pub mod model;
pub mod store;

#[cfg(test)]
mod smoke {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
