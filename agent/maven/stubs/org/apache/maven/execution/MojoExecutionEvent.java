package org.apache.maven.execution;

public class MojoExecutionEvent {
    public MavenSession getSession() {
        throw new UnsupportedOperationException();
    }

    public org.apache.maven.project.MavenProject getProject() {
        throw new UnsupportedOperationException();
    }

    public org.apache.maven.plugin.MojoExecution getExecution() {
        throw new UnsupportedOperationException();
    }

    public org.apache.maven.plugin.Mojo getMojo() {
        throw new UnsupportedOperationException();
    }
}
