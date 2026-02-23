package com.webapp.api.service;

import com.webapp.core.model.DataNode;
import com.webapp.core.http.RequestRouter;

/**
 * Search orchestration service. Imports from both core.model and core.http.
 */
public class SearchService {
    private final RequestRouter router;

    public SearchService(RequestRouter router) {
        this.router = router;
    }

    public void search(DataNode source, DataNode target) {
        router.route(source, "search_start");
        router.route(target, "search_start");
    }
}
