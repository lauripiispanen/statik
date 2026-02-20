package com.example.controller;

import com.example.db.UserRepository;
import com.example.model.User;

/**
 * This controller violates the boundary rule by importing directly from db layer.
 */
public class AdminController {
    private final UserRepository repo;

    public AdminController(UserRepository repo) {
        this.repo = repo;
    }

    public User getUser(Long id) {
        return repo.findById(id);
    }
}
