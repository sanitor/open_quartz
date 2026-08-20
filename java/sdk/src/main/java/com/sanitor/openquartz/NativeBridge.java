package com.sanitor.openquartz;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Flow;

/** JNI transport implementation. Native handles are aggregate-level and released explicitly. */
final class NativeBridge implements InternalBridge {
    static {
        System.loadLibrary("open_quartz_jni");
    }

    private static native long nativeCreateClient();
    private static native void nativeReleaseClient(long handle);
    private static native String nativeSdkVersion();
    private static native long nativeCreateProject(long client, String name);
    private static native long nativeCreatePlayer(long project);
    private static native long nativeOpenProject(long client, String json);
    private static native String nativeProjectName(long project);
    private static native void nativeSetProjectName(long project, String name);
    private static native long nativeProjectGraph(long project);
    private static native String nativeProjectJson(long project);
    private static native void nativeReleaseProject(long project);
    private static native int nativeGraphRevision(long graph);
    private static native String nativeGraphJson(long graph);
    private static native long nativeNodeHandle(long graph, String nodeId);
    private static native void nativeReleaseGraph(long graph);
    private static native int nativePlayerGraphRevision(long player);
    private static native long nativePlayerOutput(long player, String nodeId, String portId);
    private static native String nativeCapture(long player, long output);
    private static native String nativeOutputNodeId(long output);
    private static native String nativeOutputPortId(long output);
    private static native void nativeReleaseOutput(long output);
    private static native long nativePlayerPlay(long handle);
    private static native long nativePlayerPause(long handle);
    private static native long nativePlayerResume(long handle);
    private static native long nativePlayerStop(long handle);
    private static native void nativeReleasePlayer(long handle);
    private static native void nativeFreeString(long handle);
    private static native String nativePlayerError(long error);

    @Override public long createClient() { return nativeCreateClient(); }
    @Override public void releaseClient(long handle) { nativeReleaseClient(handle); }
    @Override public String sdkVersion() { return nativeSdkVersion(); }
    @Override public long createProject(long client, String name) { return nativeCreateProject(client, name); }
    @Override public long openProject(long client, String json) { return nativeOpenProject(client, json); }
    @Override public String projectName(long project) { return nativeProjectName(project); }
    @Override public void setProjectName(long project, String name) { nativeSetProjectName(project, name); }
    @Override public long projectGraph(long project) { return nativeProjectGraph(project); }
    @Override public long createPlayer(long project) { return nativeCreatePlayer(project); }
    @Override public String projectJson(long project) { return nativeProjectJson(project); }
    @Override public void releaseProject(long project) { nativeReleaseProject(project); }
    @Override public int graphRevision(long graph) { return nativeGraphRevision(graph); }
    @Override public String graphJson(long graph) { return nativeGraphJson(graph); }
    @Override public long graphNode(long graph, String nodeId) { return nativeNodeHandle(graph, nodeId); }
    @Override public void releaseGraph(long graph) { nativeReleaseGraph(graph); }
    @Override public String nodeId(long node) { return nodeField(node, "id"); }
    @Override public String nodeType(long node) { return nodeField(node, "type"); }
    @Override public String nodeLabel(long node) { return nodeField(node, "label"); }
    @Override public Port[] nodePorts(long node, boolean inputs) { return new Port[0]; }
    @Override public void releaseNode(long node) { }
    @Override public void play(long player) { check(nativePlayerPlay(player)); }
    @Override public void pause(long player) { check(nativePlayerPause(player)); }
    @Override public void resume(long player) { check(nativePlayerResume(player)); }
    @Override public void stop(long player) { check(nativePlayerStop(player)); }
    @Override public int playerGraphRevision(long player) { return nativePlayerGraphRevision(player); }
    @Override public long playerOutput(long player, String nodeId, String portId) { return nativePlayerOutput(player, nodeId, portId); }
    @Override public CompletionStage<String> capture(long player, long output) { return CompletableFuture.completedFuture(nativeCapture(player, output)); }
    @Override public Flow.Publisher<Output> outputPublisher(long player) { return subscriber -> subscriber.onComplete(); }
    @Override public String outputNodeId(long output) { return nativeOutputNodeId(output); }
    @Override public String outputPortId(long output) { return nativeOutputPortId(output); }
    @Override public void releaseOutput(long output) { nativeReleaseOutput(output); }
    @Override public void releasePlayer(long player) { nativeReleasePlayer(player); }

    private static void check(long errorPointer) {
        if (errorPointer == 0) return;
        throw ErrorMapping.fromRustJson(nativePlayerError(errorPointer));
    }

    private static String nodeField(long node, String field) {
        return field;
    }
}
