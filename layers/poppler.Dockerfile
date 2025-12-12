# Docker for building the layer for poppler support in AWS lambda
FROM public.ecr.aws/lambda/provided:al2023 AS builder

# Install required dependencies
RUN dnf -y update && \
    dnf -y install poppler poppler-utils poppler-data && \
    dnf clean all

# Copy bundling helper script
COPY ./copy-dep.sh /tmp/copy-dep.sh

# Create output bundle directory
RUN mkdir -p /bundle/opt/bin /bundle/opt/lib /bundle/opt/fonts /bundle/opt/fontconfig /bundle/opt/poppler-data

# Copy required cli tools and their dependencies
RUN chmod +x /tmp/copy-dep.sh && \
    # pdfinfo - Get details about a pdf file
    /tmp/copy-dep.sh /usr/bin/pdfinfo /bundle/opt && \
    # pdftotext - PDF text extraction
    /tmp/copy-dep.sh /usr/bin/pdftotext /bundle/opt && \
    # pdftocairo - PDF rendering
    /tmp/copy-dep.sh /usr/bin/pdftocairo /bundle/opt

# Copy fonts and Poppler data
RUN cp -r /usr/share/fonts/* /bundle/opt/fonts/ && \
    cp -r /etc/fonts/* /bundle/opt/etc/fonts && \
    cp -r /usr/share/poppler/* /bundle/opt/share/poppler-data/

# Set permissions
RUN chmod -R 755 /bundle/opt

# Zip up the bundle as the layer
RUN zip /bundle /poppler-lambda-layer.zip
