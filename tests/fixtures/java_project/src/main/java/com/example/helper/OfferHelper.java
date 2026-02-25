package com.example.helper;

/**
 * Utility for offer processing.
 * Referenced from AccountService via static method call OfferHelper.calculate()
 * without an import (same-package implicit reference).
 */
public class OfferHelper {
    public static int calculate(int base) {
        return base * 2;
    }
}
