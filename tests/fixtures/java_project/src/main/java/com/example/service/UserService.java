package com.example.service;

import com.example.model.User;
import com.example.model.Role;
import com.example.util.StringUtils;

public class UserService {
    public User createUser(String name, String email) {
        String cleanName = StringUtils.sanitize(name);
        return new User(1L, cleanName, email);
    }

    public boolean isAdmin(Role role) {
        return role.isAdmin();
    }
}
