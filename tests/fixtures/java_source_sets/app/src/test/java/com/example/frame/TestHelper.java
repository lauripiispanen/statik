package com.example.frame;

/**
 * Test helper in same package as framework but in app-test source set.
 * This should NOT create a cross-module dependency edge to framework
 * via same-package resolution when source sets prevent it.
 *
 * Without source sets, this file would falsely resolve type references
 * to FrameService/FrameUtil as same-package siblings, creating a
 * spurious app-test -> framework edge via same-package resolution.
 */
public class TestHelper {
    public String helperMethod() {
        return "test helper";
    }
}
