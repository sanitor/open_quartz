package com.sanitor.openquartz;

import java.util.Objects;

public final class Node implements AutoCloseable {
    private final InternalBridge bridge;
    private long handle;
    private boolean closed;

    Node(InternalBridge bridge, long handle) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
        this.handle = handle;
    }

    public String id() {
        ensureOpen();
        return bridge.nodeId(handle);
    }

    public String type() {
        ensureOpen();
        return bridge.nodeType(handle);
    }

    public String label() {
        ensureOpen();
        return bridge.nodeLabel(handle);
    }

    public Port[] inputs() {
        ensureOpen();
        return bridge.nodePorts(handle, true);
    }

    public Port[] outputs() {
        ensureOpen();
        return bridge.nodePorts(handle, false);
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.releaseNode(handle);
        handle = 0;
    }

    private void ensureOpen() {
        if (closed) throw new IllegalStateException("Node is closed");
    }
}
