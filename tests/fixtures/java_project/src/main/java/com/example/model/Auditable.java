package com.example.model;

/**
 * Interface for entities that track audit information.
 * Used to test interface extraction and implements resolution.
 */
public interface Auditable {
    String getCreatedBy();
    String getModifiedBy();
}
