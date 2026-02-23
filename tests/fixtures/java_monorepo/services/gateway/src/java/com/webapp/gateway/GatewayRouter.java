package com.webapp.gateway;

import com.webapp.core.http.RequestRouter;
import com.webapp.core.http.SessionManager;

/**
 * Gateway module with non-standard source root (src/java instead of src/main/java).
 * Tests that source root detection handles variant layouts.
 */
public class GatewayRouter {
    private final RequestRouter router;
    private final SessionManager sessions;

    public GatewayRouter(RequestRouter router, SessionManager sessions) {
        this.router = router;
        this.sessions = sessions;
    }

    public void forward(long nodeId, String message) {
        sessions.onConnect(nodeId);
    }
}
