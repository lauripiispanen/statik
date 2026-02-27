package com.example.app;

/**
 * Test for AppController. In app-test source set, depends on app and framework.
 */
public class AppControllerTest {
    public void testHandle() {
        AppController controller = new AppController();
        String result = controller.handle("test");
    }
}
