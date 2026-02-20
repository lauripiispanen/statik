package com.example.model;

public enum Role {
    ADMIN,
    USER,
    GUEST;

    public boolean isAdmin() {
        return this == ADMIN;
    }
}
