package com.webapp.api;

import com.webapp.api.service.UserService;
import com.webapp.core.model.NodeRegistry;

/**
 * Test file in src/test/java. Should be treated as entry point.
 */
@Test
public class UserServiceTest {
    public void testGetUser() {
        NodeRegistry registry = new NodeRegistry();
        UserService service = new UserService(registry);
        assert service.getActiveCount() == 0;
    }
}
