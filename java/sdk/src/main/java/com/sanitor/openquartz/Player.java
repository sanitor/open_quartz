package com.sanitor.openquartz;

import java.util.Objects;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Flow;

public final class Player implements AutoCloseable {
    private final InternalBridge bridge;
    private long handle;
    private boolean closed;

    Player(InternalBridge bridge, long handle) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
        this.handle = handle;
    }

    public void play() { ensureOpen(); bridge.play(handle); }
    public void pause() { ensureOpen(); bridge.pause(handle); }
    public void resume() { ensureOpen(); bridge.resume(handle); }
    public void stop() { ensureOpen(); bridge.stop(handle); }
    public int graphRevision() { ensureOpen(); return bridge.playerGraphRevision(handle); }
    public Output output(String nodeId, String portId) {
        ensureOpen();
        return new Output(bridge, handle, bridge.playerOutput(handle, nodeId, portId));
    }
    public CompletionStage<String> capture(Output output) {
        ensureOpen();
        return bridge.capture(handle, output.handle());
    }
    public Flow.Publisher<Output> outputs() {
        ensureOpen();
        return bridge.outputPublisher(handle);
    }

    @Override
    public void close() {
        if (closed) return;
        closed = true;
        bridge.releasePlayer(handle);
        handle = 0;
    }

    private void ensureOpen() {
        if (closed) throw new IllegalStateException("Player is closed");
    }
}
