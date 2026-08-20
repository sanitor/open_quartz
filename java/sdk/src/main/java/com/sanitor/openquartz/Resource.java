package com.sanitor.openquartz;

import java.util.Collections;
import java.util.Map;
import java.util.Objects;

public final class Resource {
    private final String id;
    private final String kind;
    private final Map<String, Object> source;

    public Resource(String id, String kind, Map<String, Object> source) {
        this.id = Objects.requireNonNull(id, "id");
        this.kind = Objects.requireNonNull(kind, "kind");
        this.source = Collections.unmodifiableMap(Map.copyOf(source));
    }

    public String id() { return id; }
    public String kind() { return kind; }
    public Map<String, Object> source() { return source; }
}
