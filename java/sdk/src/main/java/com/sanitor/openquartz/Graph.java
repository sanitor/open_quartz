package com.sanitor.openquartz;

import java.util.Objects;

public final class Graph implements AutoCloseable {
    private final InternalBridge bridge;
    private long handle;
    private boolean closed;

    Graph(InternalBridge bridge, long handle) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
        this.handle = handle;
    }

    public int revision() {
        ensureOpen();
        return bridge.graphRevision(handle);
    }

    public String toJson() {
        ensureOpen();
        return bridge.graphJson(handle);
    }

    public Node node(String nodeId) {
        ensureOpen();
        long nodeHandle = bridge.graphNode(handle, Objects.requireNonNull(nodeId, "nodeId"));
        return nodeHandle == 0 ? null : new Node(bridge, nodeHandle);
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.releaseGraph(handle);
        handle = 0;
    }

    private void ensureOpen() {
        if (closed) throw new IllegalStateException("Graph is closed");
    }
}
