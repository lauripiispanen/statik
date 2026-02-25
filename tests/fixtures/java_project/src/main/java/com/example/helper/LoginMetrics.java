package com.example.helper;

/**
 * Tracks login metrics.
 * Referenced from AccountService via new LoginMetrics() without an import
 * (same-package implicit reference).
 */
public class LoginMetrics {
    private int count;

    public void record() {
        count++;
    }

    public int getCount() {
        return count;
    }
}
