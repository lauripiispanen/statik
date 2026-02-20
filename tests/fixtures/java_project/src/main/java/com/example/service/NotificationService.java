package com.example.service;

import com.example.model.User;
import com.example.model.Auditable;
import com.example.model.AuditableUser;
import com.example.model.Role;
import com.example.util.StringUtils;
import com.example.db.UserRepository;

/**
 * A service with many imports to test fan-limit rules.
 * Also demonstrates inner class extraction.
 */
public class NotificationService {
    private final UserRepository repo;

    public NotificationService(UserRepository repo) {
        this.repo = repo;
    }

    public void notify(User user, String message) {
        String clean = StringUtils.sanitize(message);
        System.out.println("Notify " + user.getName() + ": " + clean);
    }

    public boolean canNotify(Role role) {
        return !role.isAdmin();
    }

    /**
     * Inner class for notification configuration.
     */
    public static class Config {
        private String template;

        public Config(String template) {
            this.template = template;
        }

        public String getTemplate() {
            return template;
        }
    }
}
