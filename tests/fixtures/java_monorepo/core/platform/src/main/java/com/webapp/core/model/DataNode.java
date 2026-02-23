package com.webapp.core.model;

/**
 * Base class for data nodes in the processing pipeline.
 * Root of the core hierarchy — heavily imported across modules.
 */
public abstract class DataNode {
    private long id;
    private String name;

    public DataNode(long id, String name) {
        this.id = id;
        this.name = name;
    }

    public long getId() { return id; }
    public String getName() { return name; }

    public abstract void process(float delta);
}
