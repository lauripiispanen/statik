package com.webapp.core.model;

import java.util.HashMap;
import java.util.Map;

/**
 * Registry for data nodes. Imported by both core internals and service modules.
 */
public class NodeRegistry {
    private final Map<Long, DataNode> nodes = new HashMap<>();

    public void register(DataNode node) {
        nodes.put(node.getId(), node);
    }

    public DataNode get(long id) {
        return nodes.get(id);
    }

    public int size() {
        return nodes.size();
    }
}
