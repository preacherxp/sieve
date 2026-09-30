package org.springframework.context;

public abstract class ApplicationEvent extends java.util.EventObject {
    public ApplicationEvent(Object source) { super(source); }
}
