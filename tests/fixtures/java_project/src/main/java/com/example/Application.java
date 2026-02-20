package com.example;

import com.example.controller.UserController;
import com.example.service.UserService;

/**
 * Main entry point for the application.
 * Should be treated as an entry point and not flagged as dead code.
 */
@SpringBootApplication
public class Application {
    public static void main(String[] args) {
        UserService service = new UserService();
        UserController controller = new UserController(service);
        System.out.println("Application started");
    }
}
