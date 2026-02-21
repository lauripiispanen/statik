use crate::model::User;

pub fn get_user() -> User {
    User::new("Bob".to_string())
}
