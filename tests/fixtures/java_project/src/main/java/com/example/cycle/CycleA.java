package com.example.cycle;

import com.example.cycle.CycleB;

public class CycleA {
    public String getValue() {
        return new CycleB().getOtherValue();
    }
}
