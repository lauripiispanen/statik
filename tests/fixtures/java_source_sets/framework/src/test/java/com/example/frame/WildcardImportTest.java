package com.example.frame;

import com.example.frame.*;

/**
 * Test that uses a wildcard import of its own package.
 *
 * The wildcard import should resolve to files within the framework source set
 * (FrameService, FrameUtil), but NOT to files in the app-test source set
 * (TestHelper) even though they share the same package name (com.example.frame).
 */
public class WildcardImportTest {
    public void testWildcard() {
        FrameService service = new FrameService();
        FrameUtil.helper();
    }
}
