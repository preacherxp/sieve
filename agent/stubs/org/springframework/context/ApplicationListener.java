package org.springframework.context;

public interface ApplicationListener<E extends ApplicationEvent> extends java.util.EventListener {
    void onApplicationEvent(E event);
}
