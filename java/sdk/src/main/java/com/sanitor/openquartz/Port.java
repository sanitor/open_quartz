package com.sanitor.openquartz;

public record Port(String nodeId, String id, String label, String dataType, Direction direction) {
    public enum Direction { INPUT, OUTPUT }
}
