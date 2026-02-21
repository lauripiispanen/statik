use rust_project::User;

#[test]
fn test_user_creation() {
    let user = User::new("Alice".to_string());
    assert_eq!(user.name, "Alice");
}
