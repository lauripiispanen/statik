package com.example.app;

import com.example.frame.FrameService;

/**
 * Application controller. Depends on framework module.
 */
public class AppController {
    private final FrameService service;

    public AppController() {
        this.service = new FrameService();
    }

    public String handle(String request) {
        return service.process(request);
    }
}
