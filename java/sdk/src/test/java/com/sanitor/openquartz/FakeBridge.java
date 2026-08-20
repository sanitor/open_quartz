package com.sanitor.openquartz;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Flow;

final class FakeBridge implements EnvironmentBridge {
    private long next = 1;
    boolean released;
    String name;

    @Override public long createClient() { return next++; }
    @Override public void releaseClient(long handle) { released = true; }
    @Override public String sdkVersion() { return "0.19.0"; }
    @Override public long createProject(long client, String name) { this.name = name; return next++; }
    @Override public long openProject(long client, String json) { this.name = "Opened"; return next++; }
    @Override public String projectName(long project) { return name; }
    @Override public void setProjectName(long project, String name) { this.name = name; }
    @Override public long projectGraph(long project) { return 2; }
    @Override public long createPlayer(long project) { return 3; }
    @Override public String projectJson(long project) { return "{\"name\":\"" + name + "\"}"; }
    @Override public void releaseProject(long project) {}
    @Override public int graphRevision(long graph) { return 1; }
    @Override public String graphJson(long graph) { return "{\"nodes\":[]}"; }
    @Override public long graphNode(long graph, String nodeId) { return "color".equals(nodeId) ? 4 : 0; }
    @Override public void releaseGraph(long graph) {}
    @Override public String nodeId(long node) { return "color"; }
    @Override public String nodeType(long node) { return "shader"; }
    @Override public String nodeLabel(long node) { return "Color"; }
    @Override public Port[] nodePorts(long node, boolean inputs) {
        return new Port[] { new Port("color", "out", "output", "sampler2D", Port.Direction.OUTPUT) };
    }
    @Override public void releaseNode(long node) {}
    @Override public void play(long player) {}
    @Override public void pause(long player) {}
    @Override public void resume(long player) {}
    @Override public void stop(long player) {}
    @Override public int playerGraphRevision(long player) { return 1; }
    @Override public long playerOutput(long player, String nodeId, String portId) { return 5; }
    @Override public CompletableFuture<String> capture(long player, long output) { return CompletableFuture.completedFuture("data:image/png;base64,test"); }
    @Override public Flow.Publisher<Output> outputPublisher(long player) { return subscriber -> subscriber.onComplete(); }
    @Override public String outputNodeId(long output) { return "renderer"; }
    @Override public String outputPortId(long output) { return "output"; }
    @Override public void releaseOutput(long output) {}
    @Override public void releasePlayer(long player) {}
    @Override public Capabilities capabilities() { return new Capabilities(true, true, true, false); }
    @Override public void setResourceResolver(ResourceResolver resolver) {}
    @Override public void setInferenceProvider(InferenceProvider provider) {}
    @Override public void setPresentationProvider(PresentationProvider provider) {}
}
