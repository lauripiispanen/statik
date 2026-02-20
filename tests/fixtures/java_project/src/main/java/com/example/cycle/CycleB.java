package com.example.cycle;

import com.example.cycle.CycleA;

public class CycleB {
    public String getOtherValue() {
        return new CycleA().getValue();
    }
}
