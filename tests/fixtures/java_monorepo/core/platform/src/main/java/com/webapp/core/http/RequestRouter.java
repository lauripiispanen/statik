package com.webapp.core.http;

import com.webapp.core.model.DataNode;

/**
 * Routes requests to data nodes. Cross-package import within the core module.
 */
public class RequestRouter {
    public void route(DataNode target, String message) {
        System.out.println("Routing to " + target.getName() + ": " + message);
    }
}
