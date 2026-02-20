package com.example;

import com.example.model.User;

/**
 * Test class with non-standard name (no "Test" suffix).
 * Should be detected as entry point via @Test annotation.
 */
public class UserVerification {
    @Test
    public void verifiesUserCreation() {
        User user = new User(1L, "Alice", "alice@test.com");
        assert user.getName().equals("Alice");
    }

    @Test
    public void verifiesUserEmail() {
        User user = new User(2L, "Bob", "bob@test.com");
        assert user.getEmail().equals("bob@test.com");
    }
}
