package com.sanitor.openquartz;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Flow;

/**
 * Testable JNI bridge implementation seam. Production supplies the native
 * implementation; Java proxy behavior remains independent of JNI availability.
 */
public final class JniBridge implements EnvironmentBridge {
    private final InternalBridge delegate;
    private ResourceResolver resourceResolver;
    private InferenceProvider inferenceProvider;
    private PresentationProvider presentationProvider;

    public JniBridge(InternalBridge delegate) {
        this.delegate = delegate;
    }

    @Override public long createClient() { return delegate.createClient(); }
    @Override public void releaseClient(long h) { delegate.releaseClient(h); }
    @Override public String sdkVersion() { return delegate.sdkVersion(); }
    @Override public long createProject(long c, String n) { return delegate.createProject(c, n); }
    @Override public long openProject(long c, String j) { return delegate.openProject(c, j); }
    @Override public String projectName(long h) { return delegate.projectName(h); }
    @Override public void setProjectName(long h, String n) { delegate.setProjectName(h, n); }
    @Override public long projectGraph(long h) { return delegate.projectGraph(h); }
    @Override public long createPlayer(long h) { return delegate.createPlayer(h); }
    @Override public String projectJson(long h) { return delegate.projectJson(h); }
    @Override public void releaseProject(long h) { delegate.releaseProject(h); }
    @Override public int graphRevision(long h) { return delegate.graphRevision(h); }
    @Override public String graphJson(long h) { return delegate.graphJson(h); }
    @Override public long graphNode(long h, String id) { return delegate.graphNode(h, id); }
    @Override public void releaseGraph(long h) { delegate.releaseGraph(h); }
    @Override public String nodeId(long h) { return delegate.nodeId(h); }
    @Override public String nodeType(long h) { return delegate.nodeType(h); }
    @Override public String nodeLabel(long h) { return delegate.nodeLabel(h); }
    @Override public Port[] nodePorts(long h, boolean inputs) { return delegate.nodePorts(h, inputs); }
    @Override public void releaseNode(long h) { delegate.releaseNode(h); }
    @Override public void play(long h) { delegate.play(h); }
    @Override public void pause(long h) { delegate.pause(h); }
    @Override public void resume(long h) { delegate.resume(h); }
    @Override public void stop(long h) { delegate.stop(h); }
    @Override public int playerGraphRevision(long h) { return delegate.playerGraphRevision(h); }
    @Override public long playerOutput(long h, String n, String p) { return delegate.playerOutput(h, n, p); }
    @Override public CompletionStage<String> capture(long p, long o) { return delegate.capture(p, o); }
    @Override public Flow.Publisher<Output> outputPublisher(long h) { return delegate.outputPublisher(h); }
    @Override public String outputNodeId(long h) { return delegate.outputNodeId(h); }
    @Override public String outputPortId(long h) { return delegate.outputPortId(h); }
    @Override public void releaseOutput(long h) { delegate.releaseOutput(h); }
    @Override public void releasePlayer(long h) { delegate.releasePlayer(h); }

    @Override public Capabilities capabilities() {
        return new Capabilities(true, true, true, inferenceProvider != null);
    }
    @Override public void setResourceResolver(ResourceResolver resolver) { resourceResolver = resolver; }
    @Override public void setInferenceProvider(InferenceProvider provider) { inferenceProvider = provider; }
    @Override public void setPresentationProvider(PresentationProvider provider) { presentationProvider = provider; }

    public CompletionStage<byte[]> resolveResource(String id) {
        return resourceResolver == null
            ? CompletableFuture.failedFuture(new IllegalStateException("Resource resolver is not configured"))
            : resourceResolver.resolve(id);
    }

    public CompletionStage<String> executeInference(String nodeId, String taskJson) {
        return inferenceProvider == null
            ? CompletableFuture.failedFuture(new IllegalStateException("Inference provider is not configured"))
            : inferenceProvider.execute(nodeId, taskJson);
    }

    public void present(String nodeId, String dataUrl) {
        if (presentationProvider == null) throw new IllegalStateException("Presentation provider is not configured");
        presentationProvider.present(nodeId, dataUrl);
    }
}
