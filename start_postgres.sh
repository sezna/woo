docker run --rm -d -i --name postgres_yalp -p 5432:5432/tcp -e POSTGRES_PASSWORD=password -v /home/alex/yalp-postgres-data:/var/lib/postgresql/data postgres:9.6
