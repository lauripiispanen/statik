package com.example.frame;

/**
 * Core framework service. Should be visible to app module.
 */
public class FrameService {
    private final FrameUtil util;

    public FrameService() {
        this.util = new FrameUtil();
    }

    public String process(String input) {
        return util.transform(input);
    }
}
