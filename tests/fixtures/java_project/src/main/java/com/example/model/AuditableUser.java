package com.example.model;

import static com.example.util.StringUtils.sanitize;

/**
 * Extends User and implements Auditable.
 * Tests extends/implements resolution and static import handling.
 */
public class AuditableUser extends User implements Auditable {
    private String createdBy;
    private String modifiedBy;

    public AuditableUser(Long id, String name, String email, String creator) {
        super(id, sanitize(name), email);
        this.createdBy = creator;
        this.modifiedBy = creator;
    }

    @Override
    public String getCreatedBy() {
        return createdBy;
    }

    @Override
    public String getModifiedBy() {
        return modifiedBy;
    }
}
