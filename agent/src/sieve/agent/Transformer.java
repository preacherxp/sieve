package sieve.agent;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassHierarchyResolver;
import java.lang.classfile.ClassModel;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.classfile.MethodModel;
import java.lang.classfile.MethodTransform;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.ClassFileTransformer;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.CodeSource;
import java.security.ProtectionDomain;
import java.util.List;
import java.util.Set;

/**
 * Adds a method-entry probe to every method of the project's own classes, and a file probe to
 * the JDK constructors and methods that open files.
 */
final class Transformer implements ClassFileTransformer {
    private static final ClassDesc PROBE = ClassDesc.of("sieve.probe.Probe");
    private static final MethodTypeDesc HIT = MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int);
    private static final MethodTypeDesc FILE = MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object);
    private static final Set<String> FILE_METHODS = Set.of("newByteChannel", "newFileChannel", "newAsynchronousFileChannel");

    private final List<Path> outputs;
    private final Set<String> jdk;

    Transformer(List<Path> outputs, Set<String> jdk) {
        this.outputs = List.copyOf(outputs);
        this.jdk = Set.copyOf(jdk);
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain, byte[] bytes) {
        if (name == null) {
            return null;
        }
        try {
            if (jdk.contains(name)) {
                return fileProbes(bytes);
            }
            Path dir = output(domain);
            // Proxies defined into an output directory's domain have no class file there.
            if (dir == null || !Files.isRegularFile(dir.resolve(name + ".class"))) {
                return null;
            }
            return methodProbes(loader, name, bytes);
        } catch (Throwable error) {
            State.error(name + ": " + error);
            return null;
        }
    }

    private Path output(ProtectionDomain domain) throws Exception {
        CodeSource source = domain == null ? null : domain.getCodeSource();
        URL location = source == null ? null : source.getLocation();
        if (location == null || !"file".equals(location.getProtocol())) {
            return null;
        }
        Path path = Path.of(location.toURI()).toAbsolutePath().normalize();
        return outputs.contains(path) ? path : null;
    }

    private static ClassFile classFile(ClassLoader loader) {
        ClassLoader resources = loader == null ? ClassLoader.getSystemClassLoader() : loader;
        return ClassFile.of(ClassFile.ClassHierarchyResolverOption.of(
                ClassHierarchyResolver.defaultResolver().orElse(ClassHierarchyResolver.ofResourceParsing(resources))));
    }

    private static byte[] methodProbes(ClassLoader loader, String name, byte[] bytes) {
        ClassFile file = classFile(loader);
        ClassModel model = file.parse(bytes);
        return file.transformClass(model, (builder, element) -> {
            if (element instanceof MethodModel method && method.code().isPresent()) {
                int id = State.method(name + "#" + method.methodName().stringValue() + method.methodType().stringValue());
                builder.transformMethod(method, MethodTransform.transformingCode(new Prologue(b -> {
                    b.loadConstant(id);
                    b.invokestatic(PROBE, "hit", HIT);
                })));
            } else {
                builder.with(element);
            }
        });
    }

    /** Reports the {@code File}, {@code String}, or {@code Path} that the first parameter opens. */
    private static byte[] fileProbes(byte[] bytes) {
        ClassFile file = classFile(null);
        ClassModel model = file.parse(bytes);
        return file.transformClass(model, (builder, element) -> {
            if (element instanceof MethodModel method && method.code().isPresent() && opens(method)) {
                builder.transformMethod(method, MethodTransform.transformingCode(new Prologue(b -> {
                    b.aload(1);
                    b.invokestatic(PROBE, "file", FILE);
                })));
            } else {
                builder.with(element);
            }
        });
    }

    private static boolean opens(MethodModel method) {
        if ((method.flags().flagsMask() & ClassFile.ACC_STATIC) != 0) {
            return false;
        }
        String name = method.methodName().stringValue();
        String type = method.methodType().stringValue();
        if (name.equals("<init>")) {
            return type.startsWith("(Ljava/io/File;");
        }
        return FILE_METHODS.contains(name) && type.startsWith("(Ljava/nio/file/Path;");
    }

    private record Prologue(java.util.function.Consumer<CodeBuilder> start) implements CodeTransform {
        @Override
        public void atStart(CodeBuilder builder) {
            start.accept(builder);
        }

        @Override
        public void accept(CodeBuilder builder, CodeElement element) {
            builder.with(element);
        }
    }
}
