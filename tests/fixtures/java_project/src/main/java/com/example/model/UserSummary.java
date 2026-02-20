package com.example.model;

/**
 * Uses same-package types without explicit imports.
 * Tests same-package implicit dependency resolution.
 */
public class UserSummary {
    private User user;
    private Role role;

    public UserSummary(User user, Role role) {
        this.user = user;
        this.role = role;
    }

    public String describe() {
        return user.getName() + " (" + role + ")";
    }
}
