package com.webapp.api.service;

import com.webapp.core.model.DataNode;
import com.webapp.core.model.NodeRegistry;

/**
 * Service layer that depends on core types.
 * This is a CROSS-MODULE import — the critical test case.
 * Without correct source root detection, this import will be unresolved.
 */
public class UserService {
    private final NodeRegistry registry;

    public UserService(NodeRegistry registry) {
        this.registry = registry;
    }

    public DataNode getUser(long id) {
        return registry.get(id);
    }

    public int getActiveCount() {
        return registry.size();
    }
}
