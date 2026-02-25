package com.example.helper;

/**
 * Uses same-package classes only via method body references:
 * - new LoginMetrics() — object creation
 * - OfferHelper.calculate() — static method call
 * No import statements needed because they are in the same package.
 */
public class AccountService {
    public void processLogin() {
        LoginMetrics metrics = new LoginMetrics();
        metrics.record();
    }

    public int calculateOffer(int base) {
        return OfferHelper.calculate(base);
    }
}
