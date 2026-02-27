package com.example.frame;

/**
 * Test for FrameService. In framework-test source set, depends on framework.
 */
public class FrameServiceTest {
    public void testProcess() {
        FrameService service = new FrameService();
        String result = service.process("hello");
    }
}
