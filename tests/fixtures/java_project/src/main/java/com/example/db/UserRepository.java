package com.example.db;

import com.example.model.User;

public class UserRepository {
    public User findById(Long id) {
        return new User(id, "test", "test@example.com");
    }

    public void save(User user) {
        // persist user
    }
}
