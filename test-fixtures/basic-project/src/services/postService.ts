import { Post, CreatePostInput, createPost } from "../models/post";
import { User } from "../models/user";
import { UserService } from "./userService";
import { Logger } from "../utils/logger";

export class PostService {
  private logger: Logger;
  private userService: UserService;
  private posts: Post[] = [];

  constructor(logger: Logger, userService: UserService) {
    this.logger = logger;
    this.userService = userService;
  }

  async createNewPost(input: CreatePostInput): Promise<Post | null> {
    const author = await this.userService.getUserById(input.authorId);
    if (!author) {
      this.logger.error(`User ${input.authorId} not found`);
      return null;
    }
    this.logger.info(`Creating post "${input.title}" by ${author.name}`);
    const post = createPost(input, author);
    this.posts.push(post);
    return post;
  }

  async getPostsByAuthor(authorId: string): Promise<Post[]> {
    return this.posts.filter((p) => p.author.id === authorId);
  }

  async getAllPosts(): Promise<Post[]> {
    this.logger.info("Fetching all posts");
    return this.posts;
  }
}
