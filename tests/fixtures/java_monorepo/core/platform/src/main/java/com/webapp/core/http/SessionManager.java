package com.webapp.core.http;

import com.webapp.core.model.NodeRegistry;

/**
 * Manages HTTP sessions. Depends on NodeRegistry (cross-package, same module).
 */
public class SessionManager {
    private final NodeRegistry registry;

    public SessionManager(NodeRegistry registry) {
        this.registry = registry;
    }

    public void onConnect(long nodeId) {
        if (registry.get(nodeId) != null) {
            System.out.println("Session started for node " + nodeId);
        }
    }
}
