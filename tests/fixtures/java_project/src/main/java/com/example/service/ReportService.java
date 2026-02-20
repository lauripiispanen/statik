package com.example.service;

import com.example.model.*;

/**
 * Uses wildcard import to pull in all model types.
 * Tests wildcard import resolution.
 */
public class ReportService {
    public String generateReport(User user, Role role) {
        return "Report for " + user.getName() + " with role " + role;
    }
}
