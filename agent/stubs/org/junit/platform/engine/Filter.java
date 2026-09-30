package org.junit.platform.engine;

public interface Filter<T> {
    FilterResult apply(T object);
}
