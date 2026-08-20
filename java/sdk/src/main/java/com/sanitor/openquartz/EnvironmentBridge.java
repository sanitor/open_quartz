package com.sanitor.openquartz;

import java.util.concurrent.CompletionStage;
import java.util.concurrent.Flow;

/** Rust SDK aggregate handle table and Java environment ports. */
public interface EnvironmentBridge extends InternalBridge {
    record Capabilities(boolean nativePresent, boolean sharedGpu, boolean externalFrames, boolean inference) {}

    Capabilities capabilities();
    void setResourceResolver(ResourceResolver resolver);
    void setInferenceProvider(InferenceProvider provider);
    void setPresentationProvider(PresentationProvider provider);

    interface ResourceResolver {
        CompletionStage<byte[]> resolve(String resourceId);
    }

    interface InferenceProvider {
        CompletionStage<String> execute(String nodeId, String taskJson);
    }

    interface PresentationProvider {
        void present(String nodeId, String dataUrl);
    }
}
