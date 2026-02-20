package com.example.util;

public class StringUtils {
    public static String sanitize(String input) {
        return input.trim().toLowerCase();
    }

    public static String capitalize(String input) {
        if (input == null || input.isEmpty()) return input;
        return input.substring(0, 1).toUpperCase() + input.substring(1);
    }
}
