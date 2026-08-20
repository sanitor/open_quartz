package com.sanitor.openquartz;

import java.util.concurrent.CompletionStage;
import java.util.concurrent.Flow;

/** Internal transport; public application code should use the object proxies. */
public interface InternalBridge {
    long createClient();
    void releaseClient(long handle);
    String sdkVersion();
    long createProject(long client, String name);
    long openProject(long client, String json);
    String projectName(long project);
    void setProjectName(long project, String name);
    long projectGraph(long project);
    long createPlayer(long project);
    String projectJson(long project);
    void releaseProject(long project);
    int graphRevision(long graph);
    String graphJson(long graph);
    long graphNode(long graph, String nodeId);
    void releaseGraph(long graph);
    String nodeId(long node);
    String nodeType(long node);
    String nodeLabel(long node);
    Port[] nodePorts(long node, boolean inputs);
    void releaseNode(long node);
    void play(long player);
    void pause(long player);
    void resume(long player);
    void stop(long player);
    int playerGraphRevision(long player);
    long playerOutput(long player, String nodeId, String portId);
    CompletionStage<String> capture(long player, long output);
    Flow.Publisher<Output> outputPublisher(long player);
    String outputNodeId(long output);
    String outputPortId(long output);
    void releaseOutput(long output);
    void releasePlayer(long player);
}
