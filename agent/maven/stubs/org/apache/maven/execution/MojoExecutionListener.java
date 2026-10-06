package org.apache.maven.execution;

public interface MojoExecutionListener {
    void beforeMojoExecution(MojoExecutionEvent event) throws org.apache.maven.plugin.MojoExecutionException;

    void afterMojoExecutionSuccess(MojoExecutionEvent event) throws org.apache.maven.plugin.MojoExecutionException;

    void afterExecutionFailure(MojoExecutionEvent event);
}
