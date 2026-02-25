package com.example.model;

import java.util.List;

public class UserMapper {
    public <T extends User> T convert(T input) { return input; }
    public List<User> toList(User[] users) { return null; }
    public void assignRoles(List<? extends Role> roles) {}
}
