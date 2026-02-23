package com.webapp.api.handler;

import com.webapp.api.service.UserService;
import com.webapp.core.http.SessionManager;

/**
 * Handles authentication requests. Imports from both api.service and core.http.
 * Tests cross-module AND cross-package resolution simultaneously.
 */
public class AuthHandler {
    private final UserService userService;
    private final SessionManager sessionManager;

    public AuthHandler(UserService userService, SessionManager sessionManager) {
        this.userService = userService;
        this.sessionManager = sessionManager;
    }

    public void handle(long userId) {
        sessionManager.onConnect(userId);
        if (userService.getUser(userId) != null) {
            System.out.println("Auth OK for " + userId);
        }
    }
}
