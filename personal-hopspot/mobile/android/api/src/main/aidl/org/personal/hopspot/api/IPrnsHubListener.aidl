package org.personal.hopspot.api;

import org.personal.hopspot.api.HubStatus;

oneway interface IPrnsHubListener {
    void onStatus(in HubStatus status);
}
