package com.webapp.codegen;

import com.webapp.core.model.DataNode;

/**
 * Code generation tool with flat source layout (src/ as source root).
 * Tests that the fallback src/ detection works for non-Maven layouts.
 */
public class SchemaGenerator {
    public String generateSchema(Class<? extends DataNode> nodeClass) {
        return "schema for " + nodeClass.getSimpleName();
    }
}
