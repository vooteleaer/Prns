package org.personal.hopspot.api;

import android.os.ParcelFileDescriptor;
import org.personal.hopspot.api.HubStatus;
import org.personal.hopspot.api.IPrnsHubListener;
import org.personal.hopspot.api.SharedInstanceInfo;

/**
 * The Personal Hopspot hub. Methods may only be appended, never reordered or changed; each
 * addition raises PrnsHubContract.API_VERSION, and a client checks apiVersion() before calling
 * anything newer than the version it needs.
 */
interface IPrnsHub {
    int apiVersion();

    HubStatus getStatus();

    SharedInstanceInfo getSharedInstanceInfo();

    /**
     * Open a shared-instance connection carrying HDLC-framed Reticulum packets, the same wire as
     * the loopback TCP listener. Returns null when the engine is not running. The caller owns the
     * returned descriptor; closing it disconnects the client. clientLabel is for diagnostics only.
     */
    ParcelFileDescriptor openLocalClient(String clientLabel);

    void registerListener(IPrnsHubListener listener);

    void unregisterListener(IPrnsHubListener listener);
}
